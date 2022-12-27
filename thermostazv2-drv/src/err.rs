pub type ThermostazvResult = anyhow::Result<()>;

#[derive(thiserror::Error, Debug)]
pub enum ThermostazvError {
    #[error("Poison error: {0}")]
    Poison(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Bincode error: {0}")]
    Bincode(String),

    #[error("Toml deserialization error: {0}")]
    TomlDe(#[from] toml::de::Error),

    #[error("Toml serialization error: {0}")]
    TomlSer(#[from] toml::ser::Error),
}
