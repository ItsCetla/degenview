#![allow(dead_code)]
#![allow(unused_imports)]

pub mod ast;
pub mod builtin_types;
pub mod lexer;
pub mod parser;
pub mod token;
pub mod type_checker;

pub use ast::*;
pub use lexer::{LexResult, Lexer};
pub use parser::Parser;
pub use token::{
    Diagnostic, DiagnosticCategory, DiagnosticSeverity, Limits, SourcePosition, SourceRange, Token,
    TokenKind, ValueType,
};
pub use type_checker::{Inferred, TypeChecker};
