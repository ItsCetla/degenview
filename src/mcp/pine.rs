//! Read-only views of the Script Manager's scripts, built on the worker thread.
//!
//! These read through `ScriptStore` (the owner of script files) and the real Pine compiler, so
//! the saved/draft distinction, content hashes and diagnostics are exactly the app's. Nothing is
//! saved, compiled into a record, run, or sent anywhere: a compile here is a pure function of
//! the source. Paths never leave this module.

use crate::format::iso8601;
use crate::mcp::bridge::McpError;
use crate::mcp::types::{
    PineCompileResult, PineCompileSummary, PineDiagnosticInfo, PineInputInfo, PineInputOption,
    PineInputsResult, PineScriptInfo, PineScriptSource, PineScriptsResult,
};
use crate::pine::compiler;
use crate::pine::program::{
    CompiledProgram, InputDefinition, InputValue, ScriptType as DeclaredType,
};
use crate::pine::token::{Diagnostic, DiagnosticCategory, DiagnosticSeverity, Limits};
use crate::script_manager::model::{CompileStatus, LocalScript, ScriptDraft, ScriptId, ScriptType};
use crate::script_manager::registry::LibraryRegistry;
use crate::script_manager::store::{COMPILER_VERSION, ScriptStore, hash_source};

/// Source returned or compiled is bounded: past this the script is not served.
pub const MAX_SOURCE_CHARS: usize = 1_000_000;
/// Diagnostics returned per compile.
const MAX_DIAGNOSTICS: usize = 200;
/// Scripts listed at most.
const MAX_SCRIPTS: usize = 500;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Version {
    Saved,
    Draft,
}

impl Version {
    pub fn label(self) -> &'static str {
        match self {
            Self::Saved => "saved",
            Self::Draft => "draft",
        }
    }
}

/// What to compile: a script's saved or draft text, or source passed in (not persisted).
pub enum Target {
    Script(String, Version),
    Inline(String),
}

fn store_error<E>(_: E) -> McpError {
    // The reason can name file paths; the caller only learns that the library was unreadable.
    McpError::AppUnavailable("The script library could not be read".into())
}

fn type_name(script_type: ScriptType) -> &'static str {
    match script_type {
        ScriptType::Indicator => "indicator",
        ScriptType::Strategy => "strategy",
        ScriptType::Library => "library",
    }
}

fn declared_name(script_type: DeclaredType) -> &'static str {
    match script_type {
        DeclaredType::Indicator => "indicator",
        DeclaredType::Strategy => "strategy",
        DeclaredType::Library => "library",
    }
}

fn status_name(status: CompileStatus) -> &'static str {
    match status {
        CompileStatus::NotCompiled => "not_compiled",
        CompileStatus::Valid => "valid",
        CompileStatus::Warning => "warning",
        CompileStatus::Error => "error",
    }
}

/// An unsaved draft that actually differs from the saved text; one that equals it is no draft.
fn real_draft(store: &mut ScriptStore, script: &LocalScript) -> Option<ScriptDraft> {
    store
        .draft(&script.id)
        .ok()
        .flatten()
        .filter(|draft| draft.source != script.source)
}

fn find_script(store: &mut ScriptStore, id: &str) -> Result<LocalScript, McpError> {
    // Scanning the library also publishes its libraries to the compiler's catalog.
    store
        .script(id)
        .map_err(store_error)?
        .ok_or_else(|| McpError::NotFound(format!("No script {id}")))
}

pub fn list_scripts(store: &mut ScriptStore) -> Result<PineScriptsResult, McpError> {
    let scripts = store.all_scripts().map_err(store_error)?;
    let total = scripts.len();
    let infos = scripts
        .iter()
        .take(MAX_SCRIPTS)
        .map(|script| {
            let hash = hash_source(&script.source);
            let has_draft = real_draft(store, script).is_some();
            PineScriptInfo {
                script_id: script.id.clone(),
                name: script.name.clone(),
                script_type: type_name(script.script_type).to_owned(),
                has_draft,
                is_favorite: script.is_favorite,
                modified_at: iso8601(script.modified_at_ms),
                last_compile: script
                    .compile_record
                    .as_ref()
                    .map(|record| PineCompileSummary {
                        status: status_name(record.status).to_owned(),
                        current: record.source_hash == hash
                            && record.compiler_version == COMPILER_VERSION,
                        compiled_at: iso8601(record.compiled_at_ms),
                    }),
                source_hash: hash,
            }
        })
        .collect();
    Ok(PineScriptsResult {
        scripts: infos,
        total,
    })
}

/// The chosen text of a script, never the other one in its place.
fn text_of(
    store: &mut ScriptStore,
    id: &str,
    version: Version,
) -> Result<(LocalScript, String, Option<ScriptDraft>), McpError> {
    let script = find_script(store, id)?;
    let draft = real_draft(store, &script);
    let source = match version {
        Version::Saved => script.source.clone(),
        Version::Draft => draft
            .as_ref()
            .map(|draft| draft.source.clone())
            .ok_or_else(|| McpError::NotFound("This script has no unsaved draft".into()))?,
    };
    if source.chars().count() > MAX_SOURCE_CHARS {
        return Err(McpError::ResourceLimit(format!(
            "The source is over {MAX_SOURCE_CHARS} characters"
        )));
    }
    Ok((script, source, draft))
}

pub fn get_script(
    store: &mut ScriptStore,
    id: &str,
    version: Version,
) -> Result<PineScriptSource, McpError> {
    let (script, source, draft) = text_of(store, id, version)?;
    Ok(PineScriptSource {
        script_id: script.id,
        name: script.name,
        script_type: type_name(script.script_type).to_owned(),
        version: version.label().to_owned(),
        size_chars: source.chars().count(),
        source_hash: hash_source(&source),
        source,
        revision_id: script.latest_revision_id,
        has_draft: draft.is_some(),
        draft_modified_at: draft.map(|draft| iso8601(draft.modified_at_ms)),
    })
}

fn severity_name(severity: DiagnosticSeverity) -> &'static str {
    match severity {
        DiagnosticSeverity::Error => "error",
        DiagnosticSeverity::Warning => "warning",
    }
}

fn category_name(category: DiagnosticCategory) -> &'static str {
    match category {
        DiagnosticCategory::Lexical => "lexical",
        DiagnosticCategory::Syntax => "syntax",
        DiagnosticCategory::Semantic => "semantic",
        DiagnosticCategory::Unsupported => "unsupported",
        DiagnosticCategory::Resource => "resource",
        DiagnosticCategory::Runtime => "runtime",
        DiagnosticCategory::Cancellation => "cancellation",
    }
}

pub fn diagnostic_info(diagnostic: &Diagnostic) -> PineDiagnosticInfo {
    PineDiagnosticInfo {
        severity: severity_name(diagnostic.severity).to_owned(),
        code: diagnostic.code.clone(),
        category: category_name(diagnostic.category).to_owned(),
        message: diagnostic.message.clone(),
        line: diagnostic.range.start.line,
        column: diagnostic.range.start.column,
        end_line: diagnostic.range.end.line,
        end_column: diagnostic.range.end.column,
    }
}

/// How an applied script's source compares with the saved script it came from now.
pub fn script_status(
    store: &mut ScriptStore,
    script_id: Option<&str>,
    applied_hash: &str,
) -> Result<String, McpError> {
    let Some(id) = script_id else {
        return Ok("unlinked".into());
    };
    Ok(match store.script(id).map_err(store_error)? {
        None => "deleted",
        Some(script) if hash_source(&script.source) == applied_hash => "unchanged",
        Some(_) => "changed",
    }
    .into())
}

fn compile_source(source: &str) -> CompiledProgram {
    compiler::compile(source, Limits::default(), &LibraryRegistry)
}

fn count(diagnostics: &[Diagnostic], severity: DiagnosticSeverity) -> usize {
    diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.severity == severity)
        .count()
}

/// Compiles without running: no trades, alerts, webhooks, saves or chart changes.
pub fn compile_script(
    store: &mut ScriptStore,
    target: Target,
) -> Result<PineCompileResult, McpError> {
    let (script_id, version, source): (Option<ScriptId>, &str, String) = match target {
        Target::Script(id, version) => {
            let (script, source, _) = text_of(store, &id, version)?;
            (Some(script.id), version.label(), source)
        }
        Target::Inline(source) => {
            if source.chars().count() > MAX_SOURCE_CHARS {
                return Err(McpError::ResourceLimit(format!(
                    "The source is over {MAX_SOURCE_CHARS} characters"
                )));
            }
            // Warm the library catalog so `import` lines resolve against the user's libraries.
            let _ = store.all_scripts();
            (None, "inline", source)
        }
    };
    let program = compile_source(&source);
    let errors = count(&program.diagnostics, DiagnosticSeverity::Error);
    let warnings = count(&program.diagnostics, DiagnosticSeverity::Warning);
    Ok(PineCompileResult {
        success: errors == 0,
        script_id,
        version: version.to_owned(),
        source_hash: hash_source(&source),
        script_type: (errors == 0)
            .then(|| declared_name(program.declaration.script_type).to_owned()),
        pine_version: program.declaration.pine_version,
        title: (errors == 0).then(|| program.declaration.title.clone()),
        error_count: errors,
        warning_count: warnings,
        diagnostics_truncated: program.diagnostics.len() > MAX_DIAGNOSTICS,
        diagnostics: program
            .diagnostics
            .iter()
            .take(MAX_DIAGNOSTICS)
            .map(diagnostic_info)
            .collect(),
    })
}

fn value_json(value: &InputValue) -> serde_json::Value {
    match value {
        InputValue::Int(value) => serde_json::json!(value),
        InputValue::Float(value) => serde_json::Number::from_f64(*value)
            .map_or(serde_json::Value::Null, serde_json::Value::Number),
        InputValue::Bool(value) => serde_json::json!(value),
        InputValue::String(value) | InputValue::Source(value) => serde_json::json!(value),
        InputValue::Color(rgba) => serde_json::json!(format!("#{rgba:08X}")),
    }
}

fn value_type(value: &InputValue) -> &'static str {
    match value {
        InputValue::Int(_) => "integer",
        InputValue::Float(_) => "float",
        InputValue::Bool(_) => "bool",
        InputValue::String(_) => "string",
        InputValue::Color(_) => "color",
        InputValue::Source(_) => "source",
    }
}

fn input_info(definition: &InputDefinition) -> PineInputInfo {
    let titles = definition.option_titles.as_deref().unwrap_or(&[]);
    PineInputInfo {
        id: definition.id.clone(),
        title: definition.title.clone(),
        value_type: value_type(&definition.default_value).to_owned(),
        default: value_json(&definition.default_value),
        min: definition.min_value,
        max: definition.max_value,
        step: definition.step,
        options: definition.options.as_ref().map(|options| {
            options
                .iter()
                .enumerate()
                .map(|(index, value)| PineInputOption {
                    value: value_json(value),
                    title: titles.get(index).cloned(),
                })
                .collect()
        }),
        group: definition.group.clone(),
        inline: definition.inline.clone(),
        tooltip: definition.tooltip.clone(),
        confirm: definition.confirm,
    }
}

/// The inputs the compiler found, straight from its own schema (no re-parsing of the text).
pub fn script_inputs(
    store: &mut ScriptStore,
    id: &str,
    version: Version,
) -> Result<PineInputsResult, McpError> {
    let (script, source, _) = text_of(store, id, version)?;
    let program = compile_source(&source);
    Ok(PineInputsResult {
        script_id: script.id,
        version: version.label().to_owned(),
        source_hash: hash_source(&source),
        compiled: program.is_valid(),
        inputs: program.input_schema.inputs.iter().map(input_info).collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::watchlist::{new_id, now_ms};
    use std::path::PathBuf;

    fn temp_store(tag: &str) -> (ScriptStore, PathBuf) {
        let root = std::env::temp_dir().join(format!("degenview-mcp-pine-{tag}-{}", new_id()));
        let store = ScriptStore::new(root.join("Scripts"), root.join("ScriptMetadata"));
        (store, root)
    }

    const WITH_INPUTS: &str = "//@version=6\nindicator(\"Inputs\", overlay=true)\n\
len = input.int(20, \"Length\", minval=1, maxval=500, step=1, group=\"Main\", tooltip=\"Bars\")\n\
mult = input.float(2.5, \"Multiplier\")\n\
show = input.bool(true, \"Show\")\n\
col = input.color(color.red, \"Colour\")\n\
plot(ta.sma(close, len) * mult)\n";

    fn create(store: &mut ScriptStore, name: &str, kind: ScriptType, source: &str) -> LocalScript {
        store.create(name, kind, Some(source), true).unwrap()
    }

    #[test]
    fn lists_scripts_with_type_draft_and_compile_state_and_no_paths() {
        let (mut store, root) = temp_store("list");
        let plain = create(
            &mut store,
            "Plain",
            ScriptType::Indicator,
            "//@version=6\nindicator(\"P\")\nplot(close)\n",
        );
        let strat = create(
            &mut store,
            "Strat",
            ScriptType::Strategy,
            "//@version=6\nstrategy(\"S\")\n",
        );
        store
            .save_draft(&ScriptDraft {
                script_id: strat.id.clone(),
                source: "//@version=6\nstrategy(\"S2\")\n".into(),
                modified_at_ms: now_ms(),
                based_on_revision_id: None,
            })
            .unwrap();
        // A draft equal to the saved text is not a draft.
        store
            .save_draft(&ScriptDraft {
                script_id: plain.id.clone(),
                source: plain.source.clone(),
                modified_at_ms: now_ms(),
                based_on_revision_id: None,
            })
            .unwrap();

        let listed = list_scripts(&mut store).unwrap();
        assert_eq!(listed.total, 2);
        let by_name = |name: &str| listed.scripts.iter().find(|s| s.name == name).unwrap();
        assert_eq!(by_name("Strat").script_type, "strategy");
        assert!(by_name("Strat").has_draft);
        assert!(!by_name("Plain").has_draft);
        assert_eq!(by_name("Plain").source_hash, hash_source(&plain.source));
        let json = serde_json::to_string(&listed).unwrap();
        assert!(
            !json.contains(root.to_str().unwrap()),
            "paths must not leak: {json}"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn saved_and_draft_are_never_substituted_for_each_other() {
        let (mut store, root) = temp_store("versions");
        let script = create(
            &mut store,
            "V",
            ScriptType::Indicator,
            "//@version=6\nindicator(\"V\")\nplot(close)\n",
        );

        let saved = get_script(&mut store, &script.id, Version::Saved).unwrap();
        assert_eq!(saved.version, "saved");
        assert_eq!(saved.source, script.source);
        assert_eq!(saved.source_hash, hash_source(&script.source));
        assert!(!saved.has_draft);
        // No draft yet: asking for it is an error, not the saved text.
        assert!(matches!(
            get_script(&mut store, &script.id, Version::Draft),
            Err(McpError::NotFound(_))
        ));

        let draft_text = "//@version=6\nindicator(\"V\")\nplot(open)\n";
        store
            .save_draft(&ScriptDraft {
                script_id: script.id.clone(),
                source: draft_text.into(),
                modified_at_ms: now_ms(),
                based_on_revision_id: None,
            })
            .unwrap();
        let draft = get_script(&mut store, &script.id, Version::Draft).unwrap();
        assert_eq!(
            (draft.version.as_str(), draft.source.as_str()),
            ("draft", draft_text)
        );
        assert_eq!(draft.source_hash, hash_source(draft_text));
        assert!(draft.has_draft && draft.draft_modified_at.is_some());
        assert_eq!(
            get_script(&mut store, &script.id, Version::Saved)
                .unwrap()
                .source,
            script.source
        );
        assert!(matches!(
            get_script(&mut store, "nope", Version::Saved),
            Err(McpError::NotFound(_))
        ));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn compiling_reports_real_diagnostics_with_positions() {
        let (mut store, root) = temp_store("compile");
        let ok = create(
            &mut store,
            "Ok",
            ScriptType::Indicator,
            "//@version=6\nindicator(\"Ok\")\nplot(close)\n",
        );
        let result =
            compile_script(&mut store, Target::Script(ok.id.clone(), Version::Saved)).unwrap();
        assert!(result.success);
        assert_eq!(result.script_type.as_deref(), Some("indicator"));
        assert_eq!(result.title.as_deref(), Some("Ok"));
        assert_eq!(result.pine_version, Some(6));
        assert_eq!((result.error_count, result.version.as_str()), (0, "saved"));

        let bad = compile_script(
            &mut store,
            Target::Inline("//@version=6\nindicator(\"B\")\nplot(\n".into()),
        )
        .unwrap();
        assert!(!bad.success);
        assert_eq!(bad.version, "inline");
        assert!(bad.script_id.is_none());
        assert!(bad.error_count > 0);
        let first = &bad.diagnostics[0];
        assert_eq!(first.severity, "error");
        assert!(first.code.starts_with("PINE"), "{}", first.code);
        assert!(first.line >= 1 && first.column >= 1);
        assert!(!first.message.is_empty());

        // The wrong version header is its own coded error.
        let old = compile_script(
            &mut store,
            Target::Inline("//@version=4\nindicator(\"X\")\n".into()),
        )
        .unwrap();
        assert!(!old.success);
        assert_eq!(old.diagnostics[0].code, "PINE0002");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn compiling_changes_nothing_on_disk() {
        let (mut store, root) = temp_store("pure");
        let script = create(
            &mut store,
            "Pure",
            ScriptType::Indicator,
            "//@version=6\nindicator(\"P\")\nplot(close)\n",
        );
        let before =
            std::fs::read_to_string(store.file_path(&script.id).unwrap().unwrap()).unwrap();
        let record_before = store.script(&script.id).unwrap().unwrap().compile_record;
        compile_script(
            &mut store,
            Target::Script(script.id.clone(), Version::Saved),
        )
        .unwrap();
        compile_script(&mut store, Target::Inline("plot(close)".into())).unwrap();
        let after = std::fs::read_to_string(store.file_path(&script.id).unwrap().unwrap()).unwrap();
        assert_eq!(before, after);
        assert_eq!(
            store.script(&script.id).unwrap().unwrap().compile_record,
            record_before
        );
        assert!(store.draft(&script.id).unwrap().is_none());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn oversized_source_is_refused() {
        let (mut store, root) = temp_store("big");
        let huge = "x".repeat(MAX_SOURCE_CHARS + 1);
        assert!(matches!(
            compile_script(&mut store, Target::Inline(huge)),
            Err(McpError::ResourceLimit(_))
        ));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn inputs_come_from_the_compilers_own_schema() {
        let (mut store, root) = temp_store("inputs");
        let script = create(&mut store, "Inputs", ScriptType::Indicator, WITH_INPUTS);
        let result = script_inputs(&mut store, &script.id, Version::Saved).unwrap();
        assert!(result.compiled);
        let ids: Vec<_> = result.inputs.iter().map(|i| i.id.as_str()).collect();
        assert_eq!(ids, ["len", "mult", "show", "col"]);

        let len = &result.inputs[0];
        assert_eq!(len.value_type, "integer");
        assert_eq!(len.default, serde_json::json!(20));
        assert_eq!(
            (len.min, len.max, len.step),
            (Some(1.0), Some(500.0), Some(1.0))
        );
        assert_eq!(len.title.as_deref(), Some("Length"));
        assert_eq!(len.group.as_deref(), Some("Main"));
        assert_eq!(len.tooltip.as_deref(), Some("Bars"));
        assert_eq!(result.inputs[1].value_type, "float");
        assert_eq!(result.inputs[1].default, serde_json::json!(2.5));
        assert_eq!(result.inputs[2].default, serde_json::json!(true));
        assert_eq!(result.inputs[3].value_type, "color");
        assert!(
            result.inputs[3]
                .default
                .as_str()
                .is_some_and(|c| c.starts_with('#') && c.len() == 9)
        );

        // A draft's inputs are the draft's, not the saved script's.
        store
            .save_draft(&ScriptDraft {
                script_id: script.id.clone(),
                source: "//@version=6\nindicator(\"D\")\nn = input.int(5, \"N\")\nplot(close)\n"
                    .into(),
                modified_at_ms: now_ms(),
                based_on_revision_id: None,
            })
            .unwrap();
        let draft = script_inputs(&mut store, &script.id, Version::Draft).unwrap();
        assert_eq!(draft.inputs.len(), 1);
        assert_eq!(draft.inputs[0].id, "n");
        let _ = std::fs::remove_dir_all(root);
    }
}
