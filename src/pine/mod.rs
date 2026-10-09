#![allow(dead_code)]
#![allow(unused_imports)]

pub mod lexer;
pub mod token;

pub use lexer::{LexResult, Lexer};
pub use token::{
    Diagnostic, DiagnosticCategory, DiagnosticSeverity, Limits, SourcePosition, SourceRange, Token,
    TokenKind, ValueType,
};
