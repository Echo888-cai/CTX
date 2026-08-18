use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Content-addressed page, optionally a named frame (virtual address).
///
/// Examples:
/// - `ctx://shell/9ba72f3c1a2e`
/// - `ctx://shell/9ba72f3c1a2e#auth::login`
/// - `ctx://file/81bfa4c2d91e#render_virtualized`
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CtxUri {
    pub kind: String,
    pub id: String,
    /// Named frame inside the page. Store keys never include this.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frame: Option<String>,
}

#[derive(Debug, Error)]
pub enum UriError {
    #[error("invalid CTX URI: {0}")]
    Invalid(String),
}

impl CtxUri {
    pub const SCHEME: &'static str = "ctx";

    pub fn new(kind: impl Into<String>, content_hash: &str) -> Self {
        let id = short_id(content_hash);
        Self {
            kind: kind.into(),
            id,
            frame: None,
        }
    }

    pub fn with_frame(mut self, name: impl Into<String>) -> Self {
        let name = name.into();
        if !name.is_empty() {
            self.frame = Some(name);
        }
        self
    }

    /// Physical page key. Frames are virtual addresses, not extra blobs.
    pub fn page_key(&self) -> String {
        format!("ctx://{}/{}", self.kind, self.id)
    }

    pub fn parse(s: &str) -> Result<Self, UriError> {
        let rest = s
            .strip_prefix("ctx://")
            .ok_or_else(|| UriError::Invalid(s.to_string()))?;
        let (path, frame) = match rest.split_once('#') {
            Some((path, frag)) if !frag.is_empty() => (path, Some(frag.to_string())),
            Some((path, _)) => (path, None),
            None => (rest, None),
        };
        let (kind, id) = path
            .split_once('/')
            .ok_or_else(|| UriError::Invalid(s.to_string()))?;
        if kind.is_empty() || id.is_empty() {
            return Err(UriError::Invalid(s.to_string()));
        }
        Ok(Self {
            kind: kind.to_string(),
            id: id.to_string(),
            frame,
        })
    }
}

impl std::fmt::Display for CtxUri {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "ctx://{}/{}", self.kind, self.id)?;
        if let Some(frame) = &self.frame {
            write!(f, "#{frame}")?;
        }
        Ok(())
    }
}

/// First 12 hex chars of a BLAKE3 digest. 48 bits is enough at personal scale.
pub fn short_id(hash: &str) -> String {
    if hash.is_ascii() {
        hash.get(..12.min(hash.len())).unwrap_or(hash).to_string()
    } else {
        hash.chars().take(12).collect()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Frame {
    pub name: String,
    pub kind: String,
    pub start_line: u32,
    pub end_line: u32,
    pub hint: String,
}

impl Frame {
    pub fn new(name: impl Into<String>, kind: impl Into<String>, start: u32, end: u32) -> Self {
        Self {
            name: name.into(),
            kind: kind.into(),
            start_line: start.max(1),
            end_line: end.max(start.max(1)),
            hint: String::new(),
        }
    }

    pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = hint.into();
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip() {
        let uri = CtxUri::new("shell", "9ba72f3c1a2eabcd");
        assert_eq!(uri.to_string(), "ctx://shell/9ba72f3c1a2e");
        assert_eq!(CtxUri::parse(&uri.to_string()).unwrap(), uri);
    }

    #[test]
    fn fragment_is_virtual_address() {
        let uri = CtxUri::parse("ctx://shell/9ba72f3c1a2e#auth::login").unwrap();
        assert_eq!(uri.kind, "shell");
        assert_eq!(uri.id, "9ba72f3c1a2e");
        assert_eq!(uri.frame.as_deref(), Some("auth::login"));
        assert_eq!(uri.page_key(), "ctx://shell/9ba72f3c1a2e");
        assert_eq!(uri.to_string(), "ctx://shell/9ba72f3c1a2e#auth::login");
        let page = CtxUri::parse(&uri.page_key()).unwrap();
        assert!(page.frame.is_none());
    }
}
