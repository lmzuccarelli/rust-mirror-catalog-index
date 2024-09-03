use custom_logger::*;
use flate2::read::GzDecoder;
use mirror_utils::FsLayer;
use std::collections::HashSet;
use std::fs;
use std::fs::File;
use std::path::Path;
use tar::Archive;

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
        log.trace(&format!("[untar_layers] cache file {}", cache_file.clone()));
        if !Path::new(&cache_file).exists() {
            let file = format!("{}/{}/{}", blobs_dir.clone(), &blob[0..2], blob);
            log.debug(&format!("[untar_layers] blobs file {}", file));
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
                    log.debug(&format!("[untar_layers] untarring file {} ", &blob[..6]));
                    let this_tar_gz =
                        File::open(file.clone()).expect("[untar_layers] could not open blob file");
                    let this_tar = GzDecoder::new(this_tar_gz);
                    let mut this_archive = Archive::new(this_tar);
                    match this_archive.unpack(cache_file.clone()) {
                        Ok(arch) => arch,
                        Err(error) => {
                            let msg = format!(
                                "[untar_layers] skipping this error : {} ",
                                &error.to_string()
                            );
                            log.warn(&msg);
                        }
                    };
                    break;
                }
            }
        } else {
            log.info(&format!("[untar_layers] cache exists {}", cache_file));
        }
    }
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
                let entry = path.expect("[find_dir] could not resolve path entry");
                let file = entry.path();
                // go down one more level
                let sub_paths = fs::read_dir(file).unwrap();
                for sub_path in sub_paths {
                    let sub_entry = sub_path.expect("[find dir] could not resolve sub path entry");
                    let sub_name = sub_entry.path();
                    let str_dir = sub_name.into_os_string().into_string().unwrap();
                    if str_dir.contains(&name) {
                        return str_dir;
                    }
                }
            }
        }
        Err(error) => {
            let msg = format!(
                "[find_dir] dir: {} {}",
                dir.clone(),
                error.to_string().to_lowercase()
            );
            log.error(&msg);
        }
    }
    return "".to_string();
}

#[cfg(test)]
mod tests {
    // this brings everything from parent's scope into this scope
    use super::*;
    use mirror_utils::FsLayer;
    use std::fs;

    macro_rules! aw {
        ($e:expr) => {
            tokio_test::block_on($e)
        };
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
            //number: None,
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
}
