use std::fs::File;
use std::fs::OpenOptions;
use std::io::Error;
use std::io::prelude::*;
use std::path::Path;

use regex::Regex;
use toml::Value;

#[derive(Debug)]
pub enum TomlError {
    Parse(#[allow(dead_code)] &'static str),
    Io(#[allow(dead_code)] Error),
}

pub fn read_version(file: String) -> Option<String> {
    let value: Value = toml::from_str(&file).ok()?;
    let version = value.get("package")?.get("version")?.as_str()?.to_string();
    Some(version).filter(|v| !v.is_empty())
}

pub fn file_with_new_version(file: String, new_version: &str) -> String {
    let re = Regex::new(r#"version\s=\s"\d+\.\d+\.\d+""#).unwrap();
    let new_version = format!("version = \"{}\"", new_version);
    re.replace(&file, &new_version[..]).to_string()
}

pub fn read_from_file(repository_path: &str) -> Result<String, TomlError> {
    let file_path = Path::new(&repository_path).join("Cargo.toml");
    let cargo_file = match read_cargo_toml(&file_path) {
        Ok(buffer) => buffer,
        Err(err) => return Err(TomlError::Io(err)),
    };

    match read_version(cargo_file) {
        Some(version) => Ok(version),
        None => Err(TomlError::Parse("No version field found")),
    }
}

pub fn write_new_version(repository_path: &str, new_version: &str) -> Result<(), Error> {
    let file_path = Path::new(&repository_path).join("Cargo.toml");
    let cargo_toml = read_cargo_toml(&file_path)?;
    let new_cargo_toml = file_with_new_version(cargo_toml, new_version);
    let mut handle = OpenOptions::new().read(true).write(true).open(file_path)?;
    handle.write_all(new_cargo_toml.as_bytes())
}

fn read_cargo_toml(file_path: &Path) -> Result<String, Error> {
    let mut handle = File::open(file_path)?;

    let mut buffer = String::new();
    match handle.read_to_string(&mut buffer) {
        Ok(_) => Ok(buffer),
        Err(err) => Err(err),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn example_file() -> String {
        "[package]
    name = \"semantic-rs\"
    version = \"0.1.0\"
    authors = [\"Jan Schulte <hello@unexpected-co.de>\"]
    [dependencies]
    term = \"0.2\"
    toml = \"0.1\""
            .to_string()
    }

    fn example_file_without_version() -> String {
        "[package]
    name = \"semantic-rs\"
    authors = [\"Jan Schulte <hello@unexpected-co.de>\"]
    [dependencies]
    term = \"0.2\"
    toml = \"0.1\""
            .to_string()
    }

    #[test]
    fn read_version_number() {
        let version_str = read_version(example_file());
        assert_eq!(version_str, Some("0.1.0".into()));
    }

    #[test]
    fn read_file_without_version_number() {
        let version_str = read_version(example_file_without_version());
        assert_eq!(version_str, None);
    }

    #[test]
    fn write_new_version_number() {
        let new_toml_file = file_with_new_version(example_file(), "0.2.0");
        let expected_file = "[package]
    name = \"semantic-rs\"
    version = \"0.2.0\"
    authors = [\"Jan Schulte <hello@unexpected-co.de>\"]
    [dependencies]
    term = \"0.2\"
    toml = \"0.1\""
            .to_string();
        assert_eq!(new_toml_file, expected_file);
    }
}
