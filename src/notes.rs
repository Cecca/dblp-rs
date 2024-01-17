use anyhow::{bail, Result};
use serde::{de::Error, Deserialize};
use serde_yaml::Error as YamlError;
use std::{
    fs::File,
    io::{Read, Write},
    path::{Path, PathBuf},
};

use crate::dblp;

pub fn create_notes_file<P: AsRef<Path>>(dir: P, bib_key: &str, title: &str) -> Result<PathBuf> {
    if let Some(existing) =
        files_with_metadata(dir.as_ref()).find(|(_path, meta)| dbg!(&meta.key) == bib_key)
    {
        eprintln!("file already existing: {:?}", existing.0);
        bail!("file already existing: {:?}", existing.0);
    }
    let title = title.replace(':', "-");
    let p = dir.as_ref().to_owned().join(title).with_extension("md");
    let entry = dblp::fetch_bibtex(bib_key)?;
    let yaml_str = serde_yaml::to_string(&entry)?;

    let mut f = File::create(&p)?;

    println!("---\nkey: {}\n{}---", bib_key, yaml_str);
    writeln!(f, "---\nkey: {}\n{}---", bib_key, yaml_str)?;
    Ok(p)
}

#[derive(Debug, Deserialize, Clone)]
pub struct ShortMetadata {
    pub title: String,
    pub key: String,
}

impl TryFrom<&str> for ShortMetadata {
    type Error = YamlError;
    fn try_from(value: &str) -> Result<Self, Self::Error> {
        let meta_str =
            get_metadata_str(value).ok_or(YamlError::custom("missing metadata header"))?;
        serde_yaml::from_str(meta_str)
    }
}

/// Returns true if the file contains a YAML header
pub fn get_metadata_str(content: &str) -> Option<&str> {
    let mut chunks = content.split("---");
    let before = chunks.next();
    let meta = chunks.next();
    let after = chunks.next();

    if before.is_none() || !before.unwrap().trim().is_empty() {
        return None;
    }
    after?;

    meta
}

pub fn files_with_metadata<P: AsRef<Path>>(
    directory: P,
) -> impl Iterator<Item = (PathBuf, ShortMetadata)> {
    use walkdir::WalkDir;
    let mut buf = String::new();
    WalkDir::new(directory)
        .into_iter()
        .filter_map(Result::ok)
        .filter_map(move |p| {
            if p.path().is_file() {
                buf.clear();
                let mut f = std::fs::File::open(p.path()).ok()?;
                f.read_to_string(&mut buf).ok()?;
                let meta = ShortMetadata::try_from(buf.as_str()).ok()?;
                Some((p.path().to_owned(), meta))
            } else {
                None
            }
        })
}

#[test]
fn test_has_metadata() {
    assert!(get_metadata_str(
        "---
        title: test
        ---
        "
    )
    .is_some());
    assert!(get_metadata_str(
        "---
        title: test
        "
    )
    .is_none());
    assert!(get_metadata_str(
        "
        title: test
        ---
    
        # This is the rest of the document
        "
    )
    .is_none());
    assert!(get_metadata_str(
        "---
        title: test
        ---
    
        # This is the rest of the document
        "
    )
    .is_some());
}
