use custom_logger::*;
use flate2::read::GzDecoder;
use mirror_config::Operator;
use mirror_copy::{get_blobs_file, FsLayer, ImageReference};
use serde_derive::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::fs::File;
use std::path::Path;
use tar::Archive;

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManifestConfig {
    pub media_type: String,
    pub size: i64,
    pub digest: String,
}

// used only for operator index manifests
#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ManifestSchema {
    pub tag: Option<String>,
    pub name: Option<String>,
    pub architecture: Option<String>,
    pub schema_version: Option<i64>,
    pub config: Option<ManifestConfig>,
    pub history: Option<Vec<History>>,
    pub fs_layers: Vec<FsLayer>,
}

#[derive(Default, Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct History {
    #[serde(rename = "v1Compatibility")]
    pub v1compatibility: String,
}

// untar layers in directory denoted by parameter 'dir'
pub async fn untar_layers(
    log: &Logging,
    blobs_dir: String,
    cache_dir: String,
    layers: Vec<FsLayer>,
) {
    // clean all duplicates
    let mut images = Vec::new();
    let mut seen = HashSet::new();
    for img in layers.iter() {
        // truncate sha256:
        let truncated_image = img.blob_sum.split(":").nth(1).unwrap();
        if !seen.contains(truncated_image) {
            seen.insert(truncated_image);
            images.push(img.blob_sum.clone());
        }
    }

    // read directory, iterate each file and untar
    for path in images.iter() {
        let blob = path.split(":").nth(1).unwrap();
        let cache_file = cache_dir.clone() + "/" + &blob[..6];
        log.trace(&format!("cache file {}", cache_file.clone()));
        if !Path::new(&cache_file).exists() {
            let file = get_blobs_file(blobs_dir.clone(), blob);
            log.trace(&format!("blobs file {}", file));
            let tar_gz = File::open(file.clone()).expect("could not open file");
            let tar = GzDecoder::new(tar_gz);
            let mut archive = Archive::new(tar);
            let entries = archive.entries();
            for entry in entries.unwrap() {
                let ent = entry.unwrap();
                let path = ent.path().unwrap();
                let path_str = path.to_str().unwrap();
                // we are really interested in either the configs or release-manifests directories
                if path_str.contains("configs/") || path_str.contains("release-manifests/") {
                    // should always be a sha256 string
                    log.info(&format!("untarring file {} ", &blob[..6]));
                    let this_tar_gz = File::open(file.clone()).expect("could not open file");
                    let this_tar = GzDecoder::new(this_tar_gz);
                    let mut this_archive = Archive::new(this_tar);
                    match this_archive.unpack(cache_file.clone()) {
                        Ok(arch) => arch,
                        Err(error) => {
                            let msg = format!("skipping this error : {} ", &error.to_string());
                            log.warn(&msg);
                        }
                    };
                    break;
                }
            }
        } else {
            log.info(&format!("cache exists {}", cache_file));
        }
    }
}

// parse_image_index - best attempt to parse image index and return catalog reference
pub fn parse_image_index(log: &Logging, operators: Vec<Operator>) -> Vec<ImageReference> {
    let mut image_refs = vec![];
    for ops in operators.iter() {
        let img = ops.catalog.clone();
        log.trace(&format!("catalogs {:#?}", img));
        let mut hld = img.split("/");
        let reg = hld.nth(0).unwrap();
        let ns = hld.nth(0).unwrap();
        let index = hld.nth(0).unwrap();
        let mut i = index.split(":");
        let name = i.nth(0).unwrap();
        let ver = i.nth(0).unwrap();
        let ir = ImageReference {
            registry: reg.to_string(),
            namespace: ns.to_string(),
            name: name.to_string(),
            version: ver.to_string(),
        };
        log.debug(&format!("image reference {:#?}", img));
        image_refs.insert(0, ir);
    }
    image_refs
}

// get_cache_dir
pub fn get_cache_dir(dir: String, name: String, version: String, arch: Option<String>) -> String {
    let mut file = dir.clone();
    file.push_str(&name);
    file.push_str(&"/");
    file.push_str(&version);
    file.push_str(&"/");
    if arch.is_some() {
        file.push_str(&arch.unwrap());
        file.push_str(&"/");
    }
    file.push_str(&"cache");
    file
}

// find a specific directory in the untar layers
pub async fn find_dir(log: &Logging, dir: String, name: String) -> String {
    let paths = fs::read_dir(&dir);
    // for both release & operator image indexes
    // we know the layer we are looking for is only 1 level
    // down from the parent
    match paths {
        Ok(res_paths) => {
            for path in res_paths {
                let entry = path.expect("could not resolve path entry");
                let file = entry.path();
                // go down one more level
                let sub_paths = fs::read_dir(file).unwrap();
                for sub_path in sub_paths {
                    let sub_entry = sub_path.expect("could not resolve sub path entry");
                    let sub_name = sub_entry.path();
                    let str_dir = sub_name.into_os_string().into_string().unwrap();
                    if str_dir.contains(&name) {
                        return str_dir;
                    }
                }
            }
        }
        Err(error) => {
            let msg = format!("{} ", error);
            log.warn(&msg);
        }
    }
    return "".to_string();
}

// parse the manifest json for operator indexes only
pub fn parse_json_manifest(data: String) -> Result<ManifestSchema, Box<dyn std::error::Error>> {
    // Parse the string of data into serde_json::ManifestSchema.
    let root: ManifestSchema = serde_json::from_str(&data)?;
    Ok(root)
}

// contruct the manifest url
pub fn get_image_manifest_url(image_ref: ImageReference) -> String {
    // return a string in the form of (example below)
    // "https://registry.redhat.io/v2/redhat/certified-operator-index/manifests/v4.12";
    let mut url = String::from("https://");
    url.push_str(&image_ref.registry);
    url.push_str(&"/v2/");
    url.push_str(&image_ref.namespace);
    url.push_str(&"/");
    url.push_str(&image_ref.name);
    url.push_str(&"/");
    url.push_str(&"manifests/");
    url.push_str(&image_ref.version);
    url
}

// utility functions - get_manifest_json
pub fn get_manifest_json_file(
    dir: String,
    name: String,
    version: String,
    arch: Option<String>,
) -> String {
    let mut file = dir.clone();
    file.push_str(&name);
    file.push_str(&"/");
    file.push_str(&version);
    file.push_str(&"/");
    if arch.is_some() {
        file.push_str(&arch.unwrap());
        file.push_str(&"/");
    }
    file.push_str(&"manifest.json");
    file
}

#[cfg(test)]
mod tests {

    use mirror_config::Operator;
    use std::fs;

    // this brings everything from parent's scope into this scope
    use super::*;

    macro_rules! aw {
        ($e:expr) => {
            tokio_test::block_on($e)
        };
    }

    #[test]
    fn get_cache_dir_pass() {
        let res = get_cache_dir(
            String::from("./test-artifacts"),
            String::from("/operator"),
            String::from("v1"),
            None,
        );
        assert_eq!(res, String::from("./test-artifacts/operator/v1/cache"));
    }

    #[test]
    fn parse_image_index_pass() {
        let log = &Logging {
            log_level: Level::INFO,
        };
        let op = Operator {
            catalog: String::from("test.registry.io/test/operator-index:v0.0.1"),
            packages: None,
        };
        let vec_op = vec![op];
        let res = parse_image_index(log, vec_op);
        assert_eq!(res.len(), 1);
        assert_eq!(res[0].registry, String::from("test.registry.io"));
    }

    #[test]
    fn untar_layers_pass() {
        let log = &Logging {
            log_level: Level::TRACE,
        };
        let mut vec_layers = vec![];
        let fslayer = FsLayer {
            blob_sum: String::from("sha256:ac202b"),
            original_ref: Some(String::from("test-ac")),
            size: Some(112),
        };
        vec_layers.insert(0, fslayer);
        aw!(untar_layers(
            log,
            String::from("test-artifacts/raw-tar-files"),
            String::from("test-artifacts/new-cache"),
            vec_layers,
        ));
        fs::remove_dir_all("test-artifacts/new-cache").expect("should delete all test directories");
    }

    #[test]
    fn find_dir_pass() {
        let log = &Logging {
            log_level: Level::INFO,
        };
        let res = aw!(find_dir(
            log,
            String::from("test-artifacts/test-index-operator/v1.0/cache"),
            String::from("configs"),
        ));
        assert_ne!(res, String::from(""));
    }

    #[test]
    fn parse_json_manifest_pass() {
        let contents = fs::read_to_string(String::from(
            "test-artifacts/test-index-operator/v1.0/manifest.json",
        ))
        .expect("Should have been able to read the file");
        let res = parse_json_manifest(contents);
        assert!(res.is_ok());
    }

    #[test]
    fn get_image_manifest_url_pass() {
        let imageref = ImageReference {
            registry: String::from("test.registry.io"),
            namespace: String::from("test"),
            name: String::from("some-operator"),
            version: String::from("v0.0.1"),
        };
        let res = get_image_manifest_url(imageref);
        assert_eq!(
            res,
            String::from("https://test.registry.io/v2/test/some-operator/manifests/v0.0.1")
        );
    }

    #[test]
    fn get_manifest_json_file_pass() {
        let dir = String::from("./test-artifacts");
        let name = String::from("/index-manifest");
        let version = String::from("v1");
        let res = get_manifest_json_file(dir, name, version, None);
        assert_eq!(
            res,
            String::from("./test-artifacts/index-manifest/v1/manifest.json")
        );
    }
}
