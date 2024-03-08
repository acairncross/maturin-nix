use maturin::*;
use std::path::PathBuf;
use structopt::clap::AppSettings;
use structopt::StructOpt;

/// Build python wheels
#[derive(Debug, StructOpt)]
struct Info {
    /// The name of the python module to create. This module name must match that of the library in
    /// the wheel or the wheel will fail when trying to import.
    #[structopt(long = "module-name")]
    module_name: String,

    /// Path to the Cargo.toml file. This file is used to provide the metadata for the python
    /// wheel. Be aware that if this points to readme file, that readme file should also be in the
    /// same folder.
    #[structopt(long = "manifest-path")]
    manifest_path: PathBuf,

    /// Use the version/s of any Python interpreters found in the environment to tag the wheel and
    /// its contents, rather than using information from the Cargo.toml.
    #[structopt(long)]
    tag_with_python: bool,
}

impl Info {
    fn meta21(&self) -> Metadata21 {
        let cargo_toml = CargoToml::from_path(&self.manifest_path).expect("manifest_file");

        // The manifest directory is only used when the target toml file points to a readme.
        let manifest_dir = self.manifest_path.parent().unwrap();

        Metadata21::from_cargo_toml(&cargo_toml, manifest_dir).expect("metadata21")
    }

    fn cargo_metadata(&self) -> cargo_metadata::Metadata {
        println!("manifest path: {:?}", &self.manifest_path);
        cargo_metadata::MetadataCommand::new()
            .manifest_path(&self.manifest_path)
            .exec()
            .unwrap()
    }
}

/// Build python wheels
#[derive(Debug, StructOpt)]
#[structopt(
    name = "maturin-nix",
    about = "Tool for building pyo3 wheels inside nix",
    global_settings(&[AppSettings::ColoredHelp, AppSettings::VersionlessSubcommands])
)]

enum Opt {
    #[structopt(name = "build")]
    /// Build the crate into wheels
    Build {
        #[structopt(flatten)]
        info: Info,

        /// The path to the rustc artifact for a library. This library must have a crate-type of
        /// "cdylib". On macOS the library should also be compiled with
        ///  "-C link-arg=-undefined -C link-arg=dynamic_lookup";
        #[structopt(long)]
        artifact_path: PathBuf,

        /// The directory to store the output wheel.
        #[structopt(long)]
        output_dir: PathBuf,
    },
}

fn parse_abi3(abi3_feature: &str) -> Option<String> {
    if abi3_feature == "abi3" {
        Some(String::from("3"))
    } else {
        abi3_feature.strip_prefix("abi3-py").map(String::from)
    }
}

fn get_tag_from_cargo_metadata(cargo_metadata: &cargo_metadata::Metadata) -> String {
    let package = &cargo_metadata.root_package().expect("root package");
    let dependencies = &package.dependencies;
    let pyo3_package = dependencies
        .iter()
        .find(|pkg| pkg.name == "pyo3")
        .expect("pyo3");
    let mut abi3_versions: Vec<_> = pyo3_package
        .features
        .iter()
        .filter_map(|feature| parse_abi3(feature))
        .collect();
    // Minimum version supported (a bit hacky, e.g. using the fact that 3 < 37, and other small
    // numbers like 2 won't appear)
    abi3_versions
        .sort_by_key(|version_string| version_string.parse::<u32>().expect("version string"));
    let min_abi3_version_string = &abi3_versions[0];
    println!("Found minimum supported Python ABI version from Cargo.toml: {}", min_abi3_version_string);

    let python_tag = format!("cp{}", min_abi3_version_string);
    let abi_tag = "abi3";
    format!("{}-{}-linux_x86_64", python_tag, abi_tag)
}

fn main() {
    let opt = Opt::from_args();

    match opt {
        Opt::Build {
            info,
            artifact_path,
            output_dir,
        } => {
            let build_wheel = |tag: &str, so_filename: &str| {
                let tag = String::from(tag);
                let mut writer = WheelWriter::new(
                    &tag,
                    &output_dir,
                    &info.meta21(),
                    &std::collections::HashMap::default(),
                    &[tag.clone()],
                )
                .expect("writer");

                writer
                    .add_file(so_filename, &artifact_path)
                    .expect("add files");

                let wheel_path = writer.finish().expect("writer finish");

                eprintln!("📦 successfuly created wheel {}", wheel_path.display());
            };

            if info.tag_with_python {
                let target = Target::current();
                let bridge = BridgeModel::Cffi;
                // Can't assume manylinux, in fact the module definitely won't be manylinux compatible if it's
                // been built with Nix
                let manylinux = Manylinux::Off;

                println!("Looking for Python interpreters...");
                let python_interpreters =
                    PythonInterpreter::find_all(&target, &bridge).expect("python_interpreter");

                for python_interpreter in &python_interpreters {
                    println!("Found {}", python_interpreter);
                }

                for python_interpreter in python_interpreters {
                    let tag = python_interpreter.get_tag(&manylinux);
                    build_wheel(
                        &tag,
                        &python_interpreter.get_library_name(&info.module_name),
                    );
                }
            } else {
                let tag = get_tag_from_cargo_metadata(&info.cargo_metadata());
                // Could bother to tag with extension (PEP 3149) e.g.
                // ".cpython-38-x86_64-linux-gnu" or ".abi3.so" but not much point
                build_wheel(&tag, &format!("{}.so", info.module_name));
            }
        }
    }
}
