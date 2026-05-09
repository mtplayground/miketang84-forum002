use std::{env, error::Error, fmt, net::SocketAddr, num::ParseIntError};

#[derive(Clone)]
pub struct Config {
    pub database_url: String,
    pub bind_addr: SocketAddr,
    pub session_secret: String,
    pub edit_window_minutes: u64,
}

impl Config {
    pub fn from_env() -> Result<Self, ConfigError> {
        load_env();
        let edit_window_minutes = required_var("EDIT_WINDOW_MINUTES")?.parse::<u64>()?;

        if edit_window_minutes == 0 {
            return Err(ConfigError::NonPositiveEditWindowMinutes);
        }

        Ok(Self {
            database_url: required_var("DATABASE_URL")?,
            bind_addr: required_var("BIND_ADDR")?.parse()?,
            session_secret: required_var("SESSION_SECRET")?,
            edit_window_minutes,
        })
    }
}

#[derive(Debug)]
pub enum ConfigError {
    MissingVar {
        name: &'static str,
        source: env::VarError,
    },
    InvalidBindAddr(std::net::AddrParseError),
    InvalidEditWindowMinutes(ParseIntError),
    NonPositiveEditWindowMinutes,
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingVar { name, .. } => write!(f, "missing required environment variable {name}"),
            Self::InvalidBindAddr(_) => write!(f, "BIND_ADDR must be a valid socket address"),
            Self::InvalidEditWindowMinutes(_) => {
                write!(f, "EDIT_WINDOW_MINUTES must be a positive integer")
            }
            Self::NonPositiveEditWindowMinutes => {
                write!(f, "EDIT_WINDOW_MINUTES must be a positive integer")
            }
        }
    }
}

impl Error for ConfigError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::MissingVar { source, .. } => Some(source),
            Self::InvalidBindAddr(source) => Some(source),
            Self::InvalidEditWindowMinutes(source) => Some(source),
            Self::NonPositiveEditWindowMinutes => None,
        }
    }
}

impl From<std::net::AddrParseError> for ConfigError {
    fn from(source: std::net::AddrParseError) -> Self {
        Self::InvalidBindAddr(source)
    }
}

impl From<ParseIntError> for ConfigError {
    fn from(source: ParseIntError) -> Self {
        Self::InvalidEditWindowMinutes(source)
    }
}

fn load_env() {
    dotenvy::dotenv().ok();
    dotenvy::from_filename_override(".env.production").ok();
}

fn required_var(name: &'static str) -> Result<String, ConfigError> {
    env::var(name).map_err(|source| ConfigError::MissingVar { name, source })
}
