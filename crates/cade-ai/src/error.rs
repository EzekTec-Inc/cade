use derive_more::{Display, From};

pub type Result<T> = core::result::Result<T, Error>;

#[derive(Debug, Display, From)]
pub enum Error {
    #[display("{_0}")]
    #[from(String, &String, &str)]
    Custom(String),

    #[display("{msg}")]
    Provider { status: u16, msg: String },

    // -- Externals
    #[display("{_0}")]
    #[from]
    Io(std::io::Error),
    #[display("{_0}")]
    #[from]
    Reqwest(reqwest::Error),
    #[display("{_0}")]
    #[from]
    SerdeJson(serde_json::Error),
}

// region:    --- Custom

impl Error {
    pub fn custom_from_err(err: impl std::error::Error) -> Self {
        Self::Custom(err.to_string())
    }

    pub fn custom(val: impl Into<String>) -> Self {
        Self::Custom(val.into())
    }
}

// endregion: --- Custom

// region:    --- Error Boilerplate

impl std::error::Error for Error {}

// endregion: --- Error Boilerplate

// region:    --- Overflow detection

impl Error {
    /// Returns `true` when this error indicates the request exceeded the
    /// provider's context window (or a similar prompt-too-long condition).
    ///
    /// This is detected heuristically from `Provider { status, msg }`:
    /// - 400-class with a body containing "context_length_exceeded"
    ///   (OpenAI / OpenAI-compatible)
    /// - 400-class with "prompt is too long" (Anthropic)
    /// - 413 (Payload Too Large) — generic
    /// - any status with "context window", "too many tokens",
    ///   "input is too long", "max_tokens" + "exceed"
    ///
    /// Callers (see `cade-server::server::api::run`) use this to drop older
    /// turns and retry once before surfacing the failure to the user.
    pub fn is_context_overflow(&self) -> bool {
        match self {
            Error::Provider { status, msg } => {
                if *status == 413 {
                    return true;
                }
                let m = msg.to_ascii_lowercase();
                m.contains("context_length_exceeded")
                    || m.contains("context length")
                    || m.contains("context window")
                    || m.contains("prompt is too long")
                    || m.contains("input is too long")
                    || m.contains("too many tokens")
                    || m.contains("maximum context")
            }
            _ => false,
        }
    }
}

// endregion: --- Overflow detection

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn provider_413_is_overflow() {
        let e = Error::Provider {
            status: 413,
            msg: "Payload Too Large".into(),
        };
        assert!(e.is_context_overflow());
    }

    #[test]
    fn openai_context_length_exceeded_is_overflow() {
        let e = Error::Provider {
            status: 400,
            msg: r#"{"error":{"code":"context_length_exceeded","message":"Too long"}}"#.into(),
        };
        assert!(e.is_context_overflow());
    }

    #[test]
    fn anthropic_prompt_is_too_long_is_overflow() {
        let e = Error::Provider {
            status: 400,
            msg: "prompt is too long: 250000 tokens > 200000 maximum".into(),
        };
        assert!(e.is_context_overflow());
    }

    #[test]
    fn rate_limit_is_not_overflow() {
        let e = Error::Provider {
            status: 429,
            msg: "rate_limited".into(),
        };
        assert!(!e.is_context_overflow());
    }

    #[test]
    fn custom_error_is_not_overflow() {
        let e = Error::Custom("boom".into());
        assert!(!e.is_context_overflow());
    }
}
