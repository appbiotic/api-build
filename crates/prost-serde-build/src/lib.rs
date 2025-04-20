use std::{
    collections::{BTreeSet, HashMap, HashSet, VecDeque},
    fs::{create_dir_all, File},
    io::BufWriter,
    path::PathBuf,
    sync::OnceLock,
};

use anyhow::{anyhow, Context};
use appbiotic_api_protogen_spec::{ExternPath, ProtoPackageSpec, ProtogenSpec};
use handlebars::Handlebars;
use heck::ToLowerCamelCase;
use prost_types::{DescriptorProto, EnumDescriptorProto};
use serde_json::json;

#[derive(serde::Deserialize)]
pub struct Config {
    pub package_name: String,
    #[serde(default)]
    pub include_dirs: Vec<PathBuf>,
    #[serde(default)]
    pub protos: Vec<ProtoSrc>,
    #[serde(default)]
    pub compile_well_known_types: bool,
    #[serde(default)]
    pub extern_paths: BTreeSet<ExternPath>,
}

#[derive(serde::Deserialize)]
pub struct ProtoSrc {
    pub files: Vec<PathBuf>,
    pub include_dir: Vec<PathBuf>,
}

pub struct ProtoType<'a> {
    pub proto_path: String,
    pub rust_path: String,
    pub def: ProtoDef<'a>,
}

pub enum ProtoDef<'a> {
    Message(&'a DescriptorProto),
    Enum(&'a EnumDescriptorProto),
}

fn prost_wkt_extern_paths() -> &'static BTreeSet<ExternPath> {
    static SET: OnceLock<BTreeSet<ExternPath>> = OnceLock::new();
    SET.get_or_init(|| serde_json::from_str(include_str!("prost-wkt-extern-paths.json")).unwrap())
}

pub fn build(
    protogen_spec: ProtogenSpec,
    package_name: &str,
    dependencies: Vec<ProtoPackageSpec>,
    rust_out_dir: PathBuf,
) -> anyhow::Result<()> {
    let prost_serde_out_rel_path = PathBuf::from("appbiotic_api_prost_serde_build");

    let prost_serde_out_path = rust_out_dir.join(prost_serde_out_rel_path);
    drop(rust_out_dir);

    create_dir_all(&prost_serde_out_path).with_context(|| {
        format!(
            "Failed to create appbiotic_api_prost_serde_build output path `{}`",
            prost_serde_out_path.to_string_lossy()
        )
    })?;

    let include_file = prost_serde_out_path.join("_include.rs");
    let descriptor_file = prost_serde_out_path.join("_descriptor.binpb");
    let proto_package_spec_file = prost_serde_out_path.join("_proto_package_spec.json");
    let metadata_rs_file = prost_serde_out_path.join("_metadata.rs");
    let index_rs_file = prost_serde_out_path.join("_index.rs");

    let rust_package = protogen_spec
        .rust
        .iter()
        .find(|x| x.name.eq(package_name))
        .ok_or_else(|| {
            anyhow!("Failed to find rust package named `{package_name}` in protogen_spec")
        })?;

    let dependencies: HashMap<String, ProtoPackageSpec> =
        HashMap::from_iter(dependencies.into_iter().map(|x| (x.name.to_owned(), x)));

    let extern_paths: HashSet<&ExternPath> = HashSet::from_iter(
        dependencies
            .iter()
            .flat_map(|x| &x.1.extern_paths)
            .chain(prost_wkt_extern_paths().iter()),
    );
    let include_dirs: Vec<&PathBuf> = Vec::from_iter(rust_package.protos.iter().map(|x| &x.dir));

    let mut prost_config = prost_build::Config::new();

    prost_config
        .type_attribute(".", "#[derive(serde::Serialize,serde::Deserialize)]")
        .type_name_domain(["."], "type.googleapis.com");

    if rust_package.compile_well_known_protos {
        prost_config.compile_well_known_types();
    }
    for ExternPath {
        proto_path,
        rust_path,
    } in extern_paths
    {
        prost_config.extern_path(proto_path.to_owned(), rust_path.to_owned());
    }

    let config = tonic_build::configure()
        .include_file(&include_file)
        .file_descriptor_set_path(&descriptor_file)
        .build_client(true)
        .build_server(true)
        .build_transport(true)
        .out_dir(&prost_serde_out_path);

    let tonic_protos: Vec<PathBuf> = rust_package
        .protos
        .iter()
        .flat_map(|x| x.files.iter().map(|f| x.dir.join(f)))
        .collect::<Vec<PathBuf>>();

    config
        .compile_protos_with_config(prost_config, &tonic_protos, include_dirs.as_slice())
        .unwrap();

    let descriptor_bytes = std::fs::read(descriptor_file).unwrap();

    let mut descriptor = <prost_wkt_build::FileDescriptorSet as prost_wkt_build::Message>::decode(
        &descriptor_bytes[..],
    )
    .unwrap();

    // NOTE(https://github.com/tokio-rs/prost/issues/880)
    let retain_files: HashSet<String> = HashSet::from_iter(
        rust_package
            .protos
            .iter()
            .flat_map(|x| x.files.iter().map(|x| x.to_string_lossy().to_string())),
    );
    descriptor.file.retain(|f| {
        retain_files.contains(f.name()) && rust_package.proto_package_name.eq(f.package())
    });

    let root_rust_path = format!("{}::prost_serde", rust_package.name.to_lower_camel_case());

    let mut types: VecDeque<ProtoType> = VecDeque::new();
    for file in &descriptor.file {
        for msg in &file.message_type {
            types.push_back(ProtoType {
                proto_path: format!(".{}", file.package()),
                rust_path: root_rust_path.to_owned(),
                def: ProtoDef::Message(msg),
            });
        }
        for enum_ in &file.enum_type {
            types.push_back(ProtoType {
                proto_path: format!(".{}", file.package()),
                rust_path: root_rust_path.to_owned(),
                def: ProtoDef::Enum(enum_),
            });
        }
    }

    let mut extern_paths: Vec<ExternPath> = Vec::new();

    while let Some(type_) = types.pop_front() {
        match type_.def {
            ProtoDef::Message(descriptor_proto) => {
                for embedded_msg in &descriptor_proto.nested_type {
                    types.push_back(ProtoType {
                        proto_path: format!("{}.{}", type_.proto_path, descriptor_proto.name()),
                        rust_path: format!(
                            "{}::{}",
                            type_.rust_path,
                            descriptor_proto.name().to_lower_camel_case()
                        ),
                        def: ProtoDef::Message(embedded_msg),
                    })
                }
                for embedded_enum in &descriptor_proto.enum_type {
                    types.push_back(ProtoType {
                        proto_path: format!("{}.{}", type_.proto_path, descriptor_proto.name()),
                        rust_path: format!(
                            "{}::{}",
                            type_.rust_path,
                            descriptor_proto.name().to_lower_camel_case()
                        ),
                        def: ProtoDef::Enum(embedded_enum),
                    })
                }

                extern_paths.push(ExternPath {
                    proto_path: format!("{}.{}", type_.proto_path, descriptor_proto.name()),
                    rust_path: format!("{}::{}", type_.rust_path, descriptor_proto.name()),
                });
            }
            ProtoDef::Enum(enum_descriptor_proto) => {
                extern_paths.push(ExternPath {
                    proto_path: format!("{}.{}", type_.proto_path, enum_descriptor_proto.name()),
                    rust_path: format!("{}::{}", type_.rust_path, enum_descriptor_proto.name()),
                });
            }
        }
    }

    let proto_package_spec = ProtoPackageSpec {
        name: package_name.to_owned(),
        extern_paths,
    };

    serde_json::to_writer_pretty(
        BufWriter::new(File::create(&proto_package_spec_file).with_context(|| {
            format!(
                "Failed to open path `{}` for writeing proto package spec",
                proto_package_spec_file.to_string_lossy()
            )
        })?),
        &proto_package_spec,
    )
    .with_context(|| {
        format!(
            "Failed to serialize proto package spec to path `{}`",
            proto_package_spec_file.to_string_lossy()
        )
    })?;

    {
        let mut handlebars = Handlebars::new();

        let templates = [
            ("index.rs", include_str!("templates/index.rs.hbs")),
            ("metadata.rs", include_str!("templates/metadata.rs.hbs")),
        ];
        for (name, tpl_str) in templates {
            handlebars
                .register_template_string(name, tpl_str)
                .with_context(|| format!("Failed to register template `{name}`"))?;
        }

        let outputs = [
            (
                "index.rs",
                json!({
                    "rust_package_rel_path": rust_package.proto_package_name.replace('.', "::")
                }),
                index_rs_file,
            ),
            ("metadata.rs", json!({}), metadata_rs_file),
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

    prost_wkt_build::add_serde(prost_serde_out_path, descriptor);

    Ok(())
}
