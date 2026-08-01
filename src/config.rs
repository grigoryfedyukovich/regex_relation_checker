use crate::charset::Interval;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::Path;
use thiserror::Error;

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Alphabet {
    #[default]
    Ascii,
    Unicode,
}

impl Alphabet {
    pub fn scalar_intervals(self) -> &'static [Interval] {
        const ASCII: &[Interval] = &[Interval {
            start: 0,
            end: 0x7f,
        }];
        const UNICODE: &[Interval] = &[
            Interval {
                start: 0,
                end: 0xd7ff,
            },
            Interval {
                start: 0xe000,
                end: 0x10ffff,
            },
        ];
        match self {
            Self::Ascii => ASCII,
            Self::Unicode => UNICODE,
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub alphabet: Alphabet,
    pub max_product_states: usize,
    pub timeout_ms: u64,
    pub max_repeat: usize,
    pub dot_matches_newline: bool,
    pub ci_exit_code: i32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            alphabet: Alphabet::Ascii,
            max_product_states: 100_000,
            timeout_ms: 5_000,
            max_repeat: 1_000,
            dot_matches_newline: false,
            ci_exit_code: 10,
        }
    }
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("failed to read configuration {path}: {source}")]
    Read {
        path: String,
        source: std::io::Error,
    },
    #[error("invalid configuration {path}: {source}")]
    Parse {
        path: String,
        source: toml::de::Error,
    },
    #[error("ci_exit_code must be between 1 and 255")]
    InvalidExitCode,
    #[error("max_product_states must be greater than zero")]
    InvalidStateLimit,
    #[error("max_repeat must be greater than zero")]
    InvalidRepeatLimit,
    #[error("timeout_ms must be greater than zero")]
    InvalidTimeout,
}

impl Config {
    /// Load, deserialize, and validate a standalone configuration file.
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        Self::validate(Self::load_raw(path)?)
    }

    /// Load and deserialize configuration before command-line or programmatic overrides.
    ///
    /// Call [`Config::validate`] after applying all overrides.
    pub fn load_raw(path: &Path) -> Result<Self, ConfigError> {
        let contents = fs::read_to_string(path).map_err(|source| ConfigError::Read {
            path: path.display().to_string(),
            source,
        })?;
        toml::from_str(&contents).map_err(|source| ConfigError::Parse {
            path: path.display().to_string(),
            source,
        })
    }

    pub fn validate(config: Self) -> Result<Self, ConfigError> {
        if !(1..=255).contains(&config.ci_exit_code) {
            return Err(ConfigError::InvalidExitCode);
        }
        if config.max_product_states == 0 {
            return Err(ConfigError::InvalidStateLimit);
        }
        if config.max_repeat == 0 {
            return Err(ConfigError::InvalidRepeatLimit);
        }
        if config.timeout_ms == 0 {
            return Err(ConfigError::InvalidTimeout);
        }
        Ok(config)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_config(contents: &str) -> std::path::PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!(
            "regexrel-config-{}-{nonce}.toml",
            std::process::id()
        ));
        fs::write(&path, contents).unwrap();
        path
    }

    #[test]
    fn defaults_are_valid() {
        Config::validate(Config::default()).unwrap();
    }

    #[test]
    fn loads_a_complete_toml_file() {
        let path = temp_config(
            r#"
alphabet = "unicode"
max_product_states = 42
timeout_ms = 77
max_repeat = 9
dot_matches_newline = true
ci_exit_code = 31
"#,
        );
        let config = Config::load(&path).unwrap();
        fs::remove_file(path).unwrap();

        assert_eq!(config.alphabet, Alphabet::Unicode);
        assert_eq!(config.max_product_states, 42);
        assert_eq!(config.timeout_ms, 77);
        assert_eq!(config.max_repeat, 9);
        assert!(config.dot_matches_newline);
        assert_eq!(config.ci_exit_code, 31);
    }

    #[test]
    fn partial_toml_uses_defaults() {
        let path = temp_config("timeout_ms = 123\n");
        let config = Config::load(&path).unwrap();
        fs::remove_file(path).unwrap();

        assert_eq!(config.timeout_ms, 123);
        assert_eq!(config.alphabet, Alphabet::Ascii);
        assert_eq!(config.max_product_states, 100_000);
    }

    #[test]
    fn rejects_unknown_keys() {
        let path = temp_config("unknown = 1\n");
        let error = Config::load(&path).unwrap_err();
        fs::remove_file(path).unwrap();
        assert!(matches!(error, ConfigError::Parse { .. }));
    }

    #[test]
    fn validates_each_nonzero_resource_limit() {
        let config = Config {
            max_product_states: 0,
            ..Config::default()
        };
        assert!(matches!(
            Config::validate(config),
            Err(ConfigError::InvalidStateLimit)
        ));

        let config = Config {
            timeout_ms: 0,
            ..Config::default()
        };
        assert!(matches!(
            Config::validate(config),
            Err(ConfigError::InvalidTimeout)
        ));

        let config = Config {
            max_repeat: 0,
            ..Config::default()
        };
        assert!(matches!(
            Config::validate(config),
            Err(ConfigError::InvalidRepeatLimit)
        ));
    }

    #[test]
    fn validates_ci_exit_code_range() {
        for invalid in [0, 256, -1] {
            let config = Config {
                ci_exit_code: invalid,
                ..Config::default()
            };
            assert!(matches!(
                Config::validate(config),
                Err(ConfigError::InvalidExitCode)
            ));
        }
    }

    #[test]
    fn load_rejects_an_invalid_standalone_file() {
        let path = temp_config("ci_exit_code = 0\n");
        let error = Config::load(&path).unwrap_err();
        fs::remove_file(path).unwrap();
        assert!(matches!(error, ConfigError::InvalidExitCode));
    }

    #[test]
    fn load_raw_defers_validation_until_after_overrides() {
        let path = temp_config("ci_exit_code = 0\n");
        let mut config = Config::load_raw(&path).unwrap();
        fs::remove_file(path).unwrap();

        assert!(matches!(
            Config::validate(config.clone()),
            Err(ConfigError::InvalidExitCode)
        ));
        config.ci_exit_code = 17;
        assert_eq!(Config::validate(config).unwrap().ci_exit_code, 17);
    }
}
