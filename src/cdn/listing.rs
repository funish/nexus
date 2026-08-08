use serde::Serialize;

use crate::storage::SharedStorage;

#[derive(Serialize)]
pub struct CdnFile {
    pub name: String,
    pub size: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub integrity: Option<String>,
}

#[derive(Serialize)]
pub struct CdnPackageListing {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    pub path: String,
    pub files: Vec<CdnFile>,
}

#[derive(Serialize)]
pub struct CdnOrgListing {
    pub name: String,
    pub packages: Vec<String>,
}

pub async fn get_directory_listing(
    storage: &SharedStorage,
    cache_base: &str,
    filepath: &str,
    package_name: &str,
    version: &str,
) -> Option<CdnPackageListing> {
    let meta = storage.get_meta(cache_base).await?;
    let files = meta.files?;

    let prefix = if filepath.is_empty() {
        String::new()
    } else {
        format!("{filepath}/")
    };

    let mut filtered: Vec<CdnFile> = files
        .iter()
        .filter(|f| f.name.starts_with(&prefix))
        .map(|f| CdnFile {
            name: f.name[prefix.len()..].to_string(),
            size: f.size,
            integrity: f.integrity.clone(),
        })
        .filter(|f| !f.name.is_empty())
        .collect();

    filtered.sort_by(|a, b| a.name.cmp(&b.name));

    Some(CdnPackageListing {
        name: Some(package_name.to_string()),
        version: Some(version.to_string()),
        path: filepath.to_string(),
        files: filtered,
    })
}
