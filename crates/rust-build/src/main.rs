use std::{
    collections::BTreeMap,
    env,
    fs::{create_dir_all, File},
    io::{BufReader, BufWriter, Write},
    path::PathBuf,
    process::ExitCode,
};

use anyhow::{anyhow, ensure, Context};
use appbiotic_api_protogen_spec::ProtogenSpec;
use clap::Parser;
use handlebars::Handlebars;
use serde_json::json;

/// Code generator for Rust APIs
#[derive(clap::Parser)]
#[command(version, about, long_about = None)]
struct Cli {
    #[clap(subcommand)]
    cmd: Command,
}

#[derive(clap::Subcommand)]
enum Command {
    Package(PackageCommand),
}

#[derive(clap::Args)]
struct PackageCommand {
    /// Path to the protogen.json spec file
    #[clap(long, value_name = "FILE")]
    protogen_path: PathBuf,

    /// Path to the protogen.json spec file
    #[clap(long, value_name = "FILE")]
    protofetch_path: PathBuf,

    /// The name of the package to generate as specified in `protogen-path`
    /// spec file.
    #[clap(long)]
    package: String,

    /// Set to true to skip writing to `output-path` and instead write a patch
    /// to standard out.
    #[clap(long, default_value_t = false)]
    dry_run: bool,
}

#[derive(serde::Deserialize, serde::Serialize)]
struct PackageSpec {
    pub name: String,
    pub version: String,
    pub path: PathBuf,
    pub protos: Vec<PathBuf>,
    #[serde(default)]
    pub api_dependencies: Vec<String>,
}

#[derive(serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
struct CargoManifest {
    package: CargoPackage,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    features: BTreeMap<String, Vec<String>>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    dependencies: BTreeMap<String, CargoPackageDep>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    build_dependencies: BTreeMap<String, CargoPackageDep>,
}

#[derive(serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
struct CargoPackage {
    name: String,
    version: String,
    edition: String,
}

#[derive(serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "kebab-case")]
struct CargoPackageDep {
    version: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    optional: bool,
    #[serde(default)]
    features: Vec<String>,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    workspace: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    path: Option<PathBuf>,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run_cmd(cli.cmd) {
        Ok(_) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error:?}");
            ExitCode::FAILURE
        }
    }
}

fn run_cmd(cmd: Command) -> anyhow::Result<()> {
    match cmd {
        Command::Package(package) => build_package(package),
    }
}

fn build_package(package_cmd: PackageCommand) -> anyhow::Result<()> {
    let protogen_path = env::current_dir()
        .as_ref()
        .unwrap()
        .join(package_cmd.protogen_path);

    let protogen: ProtogenSpec = serde_json::from_reader(BufReader::new(
        File::open(&protogen_path).with_context(|| {
            format!(
                "Failed to open path `{}` for parsing protogen spec",
                protogen_path.to_string_lossy()
            )
        })?,
    ))
    .with_context(|| {
        format!(
            "Failed to parse protogen spec at path `{}`",
            protogen_path.to_string_lossy()
        )
    })?;

    let package_spec = protogen
        .rust
        .iter()
        .find(|x| x.name.eq(&package_cmd.package))
        .ok_or_else(|| {
            anyhow!(
                "Failed to find package `{}` in protogen file at path `{}`",
                package_cmd.package,
                protogen_path.to_string_lossy()
            )
        })?;

    ensure!(
        package_spec.path.is_relative(),
        "Package spec path for `{}` was not relative",
        package_spec.name
    );
    let package_spec_src_path = package_spec.path.join("src");
    create_dir_all(&package_spec_src_path).with_context(|| {
        format!(
            "Failed to create package source path `{}`",
            package_spec_src_path.to_string_lossy()
        )
    })?;

    let mut rel_protogen_path = PathBuf::default();
    package_spec
        .path
        .iter()
        .for_each(|_| rel_protogen_path = rel_protogen_path.join(".."));
    rel_protogen_path = rel_protogen_path.join(
        protogen_path
            .file_name()
            .context("Expected file_name from protogen_path")?,
    );

    // let tmp_dir = tempfile::Builder::new()
    //     .prefix("rust-build")
    //     .tempdir()
    //     .context("Failed to create tempdir with prefix `rust-build`")?;

    let mut manifest: CargoManifest = toml::from_str(include_str!("package_template.toml"))
        .context("Failed to decode package_template.toml")?;

    manifest.package.name = package_spec.name.to_owned();
    manifest.package.version = package_spec.version.to_owned();

    create_dir_all(&package_spec.path).context("Failed to create package_spec parent path")?;

    {
        let manifest_path = package_spec.path.join("Cargo.toml");
        let mut manifest_out = BufWriter::new(File::create(&manifest_path).with_context(|| {
            format!(
                "Failed to open path `{}` for writing package manifest",
                manifest_path.to_string_lossy()
            )
        })?);
        write!(
            manifest_out,
            "{}",
            toml::to_string_pretty(&manifest).context("Failed to serialize cargo manifest toml")?
        )
        .with_context(|| {
            format!(
                "Failed to write cargo manifest toml to path `{}`",
                manifest_path.to_string_lossy()
            )
        })?;
    }

    {
        let package_spec_path = package_spec_src_path.join("package_spec.json");
        serde_json::to_writer_pretty(
            BufWriter::new(File::create(&package_spec_path).with_context(|| {
                format!(
                    "Failed to open path `{}` for writing package spec",
                    package_spec_path.to_string_lossy()
                )
            })?),
            &package_spec,
        )
        .with_context(|| {
            format!(
                "Failed to write package spec to path `{}`",
                package_spec_path.to_string_lossy()
            )
        })?;
    }

    let mut handlebars = Handlebars::new();

    {
        let templates = [
            ("lib.rs", include_str!("templates/lib.rs.hbs")),
            ("build.rs", include_str!("templates/build.rs.hbs")),
            (
                "prost_serde.rs",
                include_str!("templates/prost_serde.rs.hbs"),
            ),
        ];
        for (name, tpl_str) in templates {
            handlebars
                .register_template_string(name, tpl_str)
                .with_context(|| format!("Failed to register template `{name}`"))?;
        }
    }

    {
        let outputs = [
            (
                "build.rs",
                json!({
                    "rel_protogen_path": rel_protogen_path.to_string_lossy().as_ref()
                }),
                package_spec.path.join("build.rs"),
            ),
            ("lib.rs", json!({}), package_spec_src_path.join("lib.rs")),
            (
                "prost_serde.rs",
                json!({}),
                package_spec_src_path.join("prost_serde.rs"),
            ),
        ];

        for (name, data, path) in outputs {
            handlebars
                .render_to_write(
                    name,
                    &data,
                    BufWriter::new(File::create(&path).with_context(|| {
                        format!(
                            "Failed to open path `{}` for writing {name} file",
                            path.to_string_lossy()
                        )
                    })?),
                )
                .with_context(|| {
                    format!(
                        "Failed to render {name} template to path `{}`",
                        path.to_string_lossy()
                    )
                })?;
        }
    }

    Ok(())
}
