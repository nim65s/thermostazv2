use thermostazv2_lib::TError;

pub type ThermostazvResult = anyhow::Result<()>;

#[derive(thiserror::Error, Debug)]
pub enum ThermostazvError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Toml deserialization error: {0}")]
    TomlDe(#[from] toml::de::Error),

    #[error("Toml serialization error: {0}")]
    TomlSer(#[from] toml::ser::Error),

    #[error("Thermostazv lib error: {0}")]
    TError(#[from] TError),
}
