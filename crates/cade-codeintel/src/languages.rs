#![allow(unsafe_code)]
/// Language detection and tree-sitter grammar registry.
use std::path::Path;

// region:    --- Language detection

/// A supported programming language.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Language {
    Rust,
    Python,
    TypeScript,
    JavaScript,
    Go,
    Unknown,
}

impl Language {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Rust       => "rust",
            Self::Python     => "python",
            Self::TypeScript => "typescript",
            Self::JavaScript => "javascript",
            Self::Go         => "go",
            Self::Unknown    => "unknown",
        }
    }
}

/// Detect the language of a file by extension.
pub fn detect_language(path: &Path) -> Language {
    match path.extension().and_then(|e| e.to_str()) {
        Some("rs")          => Language::Rust,
        Some("py")          => Language::Python,
        Some("ts") | Some("tsx") => Language::TypeScript,
        Some("js") | Some("jsx") => Language::JavaScript,
        Some("go")          => Language::Go,
        _                   => Language::Unknown,
    }
}

// endregion: --- Language detection

// region:    --- Grammar registry

/// Returns the tree-sitter grammar for the given language, if available.
pub fn get_grammar(lang: Language) -> Option<tree_sitter::Language> {
    match lang {
        #[cfg(feature = "lang-rust")]
        Language::Rust       => Some(tree_sitter_rust::language()),

        #[cfg(feature = "lang-python")]
        Language::Python     => Some(tree_sitter_python::language()),

        // TypeScript grammar uses an incompatible Language wrapper in ts 0.23 —
        // disabled until the ecosystem stabilises.
        Language::TypeScript => None,

        #[cfg(feature = "lang-javascript")]
        Language::JavaScript => Some(tree_sitter_javascript::language()),

        #[cfg(feature = "lang-go")]
        Language::Go         => Some(tree_sitter_go::language()),

        _ => None,
    }
}

// endregion: --- Grammar registry

// region:    --- Symbol kind mapping

/// Canonical symbol kind names (stable across languages).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SymbolKind {
    Function,
    Method,
    Struct,
    Enum,
    Trait,
    Interface,
    Class,
    Const,
    Type,
    Module,
    Variable,
    Field,
    Other,
}

impl SymbolKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Function  => "fn",
            Self::Method    => "method",
            Self::Struct    => "struct",
            Self::Enum      => "enum",
            Self::Trait     => "trait",
            Self::Interface => "interface",
            Self::Class     => "class",
            Self::Const     => "const",
            Self::Type      => "type",
            Self::Module    => "module",
            Self::Variable  => "variable",
            Self::Field     => "field",
            Self::Other     => "other",
        }
    }
}

// endregion: --- Symbol kind mapping

// region:    --- Tests

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn test_detect_language_rust() {
        // -- Exec
        let lang = detect_language(&PathBuf::from("foo.rs"));
        // -- Check
        assert_eq!(lang, Language::Rust);
    }

    #[test]
    fn test_detect_language_python() {
        // -- Exec
        let lang = detect_language(&PathBuf::from("bar.py"));
        // -- Check
        assert_eq!(lang, Language::Python);
    }

    #[test]
    fn test_detect_language_unknown() {
        // -- Exec
        let lang = detect_language(&PathBuf::from("README.md"));
        // -- Check
        assert_eq!(lang, Language::Unknown);
    }
}

// endregion: --- Tests
