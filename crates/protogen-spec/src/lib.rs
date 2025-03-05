use std::path::PathBuf;

#[derive(serde::Deserialize, serde::Serialize)]
pub struct ProtogenSpec {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rust: Vec<RustPackage>,
}

#[derive(serde::Deserialize, serde::Serialize)]
pub struct RustPackage {
    pub name: String,
    pub version: String,
    pub path: PathBuf,
    pub proto_package_name: String,
    #[serde(default)]
    pub compile_well_known_protos: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub protos: Vec<ProtoSrc>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub protogen_dependencies: Vec<String>,
}

#[derive(serde::Deserialize, serde::Serialize)]
pub struct ProtoSrc {
    pub dir: PathBuf,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<PathBuf>,
}

#[derive(serde::Deserialize, serde::Serialize)]
pub struct ProtoPackageSpec {
    pub name: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extern_paths: Vec<ExternPath>,
}

#[derive(Hash, PartialEq, Eq, PartialOrd, Ord, serde::Deserialize, serde::Serialize)]
pub struct ExternPath {
    pub proto_path: String,
    pub rust_path: String,
}
