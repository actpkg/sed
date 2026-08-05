//! ACT component wrapping the uutils implementation of `sed`.
//!
//! Upstream exposes no in-memory entry point: `process_file` and the only
//! non-file `LineReader` constructor are private, and non-in-place output is
//! written to stdout, which a component cannot capture. Text transforms
//! therefore run through a scratch file that is edited in place and read back.
//! That is why even the plain-text `sed` tool needs a `wasi:filesystem` grant.

use act_sdk::prelude::*;
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};

use ::sed::sed::command::{ByteSpace, CharacterMode, ProcessingContext};
use ::sed::sed::compiler::compile;
use ::sed::sed::processor::process_all_files;
use ::sed::sed::script_line_provider::ScriptValue;

fn default_true() -> bool {
    true
}

fn default_line_length() -> u32 {
    70
}

fn default_scratch_dir() -> String {
    "/tmp".to_string()
}

/// How input bytes are interpreted; mirrors sed's locale handling.
#[derive(Deserialize, JsonSchema, Default, Clone, Copy)]
#[serde(rename_all = "lowercase")]
enum CharMode {
    /// Interpret data as UTF-8 characters (default).
    #[default]
    Utf8,
    /// Interpret data as raw bytes, like the C/POSIX locale.
    Byte,
}

impl From<CharMode> for CharacterMode {
    fn from(m: CharMode) -> Self {
        match m {
            CharMode::Utf8 => CharacterMode::Utf8,
            CharMode::Byte => CharacterMode::Byte,
        }
    }
}

/// Options shared by both tools; each maps to a sed command-line flag.
#[derive(Deserialize, JsonSchema)]
struct Flags {
    /// Reject the `e` (shell), `r` (read file) and `w` (write file) commands
    /// when the script is compiled, before any input is read. Defaults to true;
    /// set false only when the script legitimately needs file I/O.
    #[serde(default = "default_true")]
    sandbox: bool,
    /// Suppress automatic printing of the pattern space (`-n`).
    #[serde(default)]
    quiet: bool,
    /// Use extended regular expressions (`-E`).
    #[serde(default)]
    extended_regexp: bool,
    /// Consider each input file separately rather than as one stream (`-s`).
    #[serde(default)]
    separate: bool,
    /// Disable GNU extensions (`--posix`).
    #[serde(default)]
    posix: bool,
    /// Separate lines by NUL bytes instead of newlines (`-z`).
    #[serde(default)]
    null_data: bool,
    /// Line-wrap length for the `l` command (`-l`).
    #[serde(default = "default_line_length")]
    line_length: u32,
    /// Annotate program execution (`--debug`).
    #[serde(default)]
    debug: bool,
    /// Whether data is interpreted as UTF-8 characters or raw bytes.
    #[serde(default)]
    character_mode: CharMode,
}

#[derive(Deserialize, JsonSchema)]
struct SedArgs {
    /// The sed program, e.g. `s/foo/bar/g`. Separate multiple commands with
    /// `;` or newlines.
    script: String,
    /// Text to transform.
    input: String,
    #[serde(flatten)]
    flags: Flags,
}

#[derive(Deserialize, JsonSchema)]
struct SedFilesArgs {
    /// The sed program, e.g. `s/foo/bar/g`.
    script: String,
    /// Paths of the files to process. Must be non-empty.
    paths: Vec<String>,
    /// Edit the files in place instead of returning the transformed text.
    #[serde(default)]
    in_place: bool,
    /// Backup suffix to keep the original under when editing in place
    /// (e.g. `.bak`). Implies `in_place`.
    #[serde(default)]
    in_place_suffix: Option<String>,
    #[serde(flatten)]
    flags: Flags,
}

/// Call metadata.
#[derive(Deserialize, JsonSchema)]
struct Meta {
    /// Guest directory used for scratch files. Must be writable.
    #[serde(default = "default_scratch_dir", rename = "scratch-dir")]
    scratch_dir: String,
}

#[derive(Serialize, JsonSchema)]
struct SedFilesResult {
    /// Transformed text. Omitted when editing in place.
    #[serde(skip_serializing_if = "Option::is_none")]
    output: Option<String>,
    /// Paths that were edited. Omitted when not editing in place.
    #[serde(skip_serializing_if = "Option::is_none")]
    edited: Option<Vec<String>>,
}

/// Build a `ProcessingContext` from the tool flags.
///
/// `ProcessingContext` derives `Default`, so only the fields that differ from
/// the zero value need setting. `length` and `hold.has_newline` are spelled out
/// because upstream's own `build_context` uses non-zero defaults for them.
fn make_context(
    flags: &Flags,
    in_place: bool,
    in_place_suffix: Option<String>,
) -> ProcessingContext {
    ProcessingContext {
        debug: flags.debug,
        regex_extended: flags.extended_regexp,
        in_place,
        in_place_suffix,
        length: flags.line_length as usize,
        quiet: flags.quiet,
        posix: flags.posix,
        separate: flags.separate,
        sandbox: flags.sandbox,
        null_data: flags.null_data,
        character_mode: flags.character_mode.into(),
        hold: ByteSpace {
            content: Vec::new(),
            has_newline: true,
        },
        ..Default::default()
    }
}

/// Report a scratch-directory problem with the grant that would fix it, rather
/// than surfacing a bare errno the caller cannot act on.
fn scratch_error(dir: &str, e: &std::io::Error) -> ActError {
    ActError::invalid_args(format!(
        "scratch directory {dir} is not usable ({e}). Grant it with:\n  \
         --grant '{{\"wasi:filesystem\":{{\"mode\":\"allowlist\",\"allow\":[{{\"path\":\"{dir}/**\",\"mode\":\"rw\"}}]}}}}'\n\
         or point the component elsewhere with metadata scratch-dir=<writable dir>."
    ))
}

/// Run `script` over `input`, returning the transformed bytes.
///
/// The input is written to a scratch file which sed then edits in place; the
/// result is read back and the scratch file removed, including on error paths.
fn run_in_scratch(
    script: &str,
    input: &[u8],
    flags: &Flags,
    scratch_dir: &str,
) -> ActResult<Vec<u8>> {
    let tmp = tempfile::Builder::new()
        .prefix("act-sed-")
        .tempfile_in(scratch_dir)
        .map_err(|e| scratch_error(scratch_dir, &e))?;
    let path = tmp.path().to_path_buf();

    let result = (|| {
        fs::write(&path, input).map_err(|e| scratch_error(scratch_dir, &e))?;
        run_over_paths(script, std::slice::from_ref(&path), flags, true, None)?;
        fs::read(&path)
            .map_err(|e| ActError::internal(format!("cannot read back scratch file: {e}")))
    })();

    // Dropping the NamedTempFile unlinks the path, so cleanup happens on the
    // error path too. sed's in-place rename replaces the inode underneath it,
    // which is why the result is read before the drop.
    drop(tmp);
    result
}

/// Compile `script` and run it over `paths`.
fn run_over_paths(
    script: &str,
    paths: &[PathBuf],
    flags: &Flags,
    in_place: bool,
    in_place_suffix: Option<String>,
) -> ActResult<()> {
    // process_all_files computes `files.len() - 1` and would panic on an empty
    // slice.
    if paths.is_empty() {
        return Err(ActError::invalid_args("no input paths given"));
    }

    let mut ctx = make_context(flags, in_place, in_place_suffix);

    // Compile errors include --sandbox rejections of e/r/w, so they are caller
    // errors: pass upstream's message through verbatim so the script can be
    // corrected.
    let commands = compile(vec![ScriptValue::StringVal(script.to_string())], &mut ctx)
        .map_err(|e| ActError::invalid_args(format!("sed: {e}")))?;

    process_all_files(commands, paths.to_vec(), &mut ctx)
        .map_err(|e| ActError::internal(format!("sed: {e}")))
}

/// Read a file, mapping io errors onto ACT error kinds.
fn read_input_file(path: &str) -> ActResult<Vec<u8>> {
    fs::read(Path::new(path)).map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => ActError::not_found(format!("file not found: {path}")),
        std::io::ErrorKind::PermissionDenied => {
            ActError::capability_denied(format!("permission denied: {path}"))
        }
        _ => ActError::internal(format!("cannot read {path}: {e}")),
    })
}

fn decode_utf8(bytes: Vec<u8>, hint: &str) -> ActResult<String> {
    String::from_utf8(bytes).map_err(|_| {
        ActError::invalid_args(format!(
            "sed produced output that is not valid UTF-8. {hint}"
        ))
    })
}

#[act_component]
mod component {
    use super::*;

    /// Transform a string with a sed script. Needs only a writable scratch
    /// directory, not access to any of your files.
    #[act_tool(
        description = "Transform text with a sed script (text in, text out). \
                       By default the script is compiled in sandbox mode, which \
                       rejects the e, r and w commands.",
        read_only
    )]
    fn sed(#[args] args: SedArgs, ctx: &mut ActContext<Meta>) -> ActResult<String> {
        let scratch = ctx.metadata().scratch_dir.clone();
        let out = run_in_scratch(&args.script, args.input.as_bytes(), &args.flags, &scratch)?;
        decode_utf8(out, "Use sed_files for binary data.")
    }

    /// Run a sed script over files on disk.
    ///
    /// Each file is processed independently, equivalent to sed's `-s`. Line
    /// numbers and `$` therefore apply per file rather than across the whole
    /// set; pass the text to `sed` directly if you need continuous-stream
    /// semantics.
    #[act_tool(
        description = "Run a sed script over files on disk, either returning the \
                       transformed text or editing the files in place. Each file \
                       is processed independently (sed -s)."
    )]
    fn sed_files(
        #[args] args: SedFilesArgs,
        ctx: &mut ActContext<Meta>,
    ) -> ActResult<SedFilesResult> {
        if args.paths.is_empty() {
            return Err(ActError::invalid_args("paths must not be empty"));
        }

        // A backup suffix is meaningless unless the file is being rewritten.
        let in_place = args.in_place || args.in_place_suffix.is_some();

        if in_place {
            let paths: Vec<PathBuf> = args.paths.iter().map(PathBuf::from).collect();
            // Fail before mutating anything if an input is missing.
            for p in &paths {
                if !p.exists() {
                    return Err(ActError::not_found(format!(
                        "file not found: {}",
                        p.display()
                    )));
                }
            }
            run_over_paths(
                &args.script,
                &paths,
                &args.flags,
                true,
                args.in_place_suffix.clone(),
            )?;
            return Ok(SedFilesResult {
                output: None,
                edited: Some(args.paths.clone()),
            });
        }

        let scratch = ctx.metadata().scratch_dir.clone();
        let mut out = Vec::new();
        for path in &args.paths {
            let input = read_input_file(path)?;
            out.extend(run_in_scratch(&args.script, &input, &args.flags, &scratch)?);
        }

        Ok(SedFilesResult {
            output: Some(decode_utf8(
                out,
                "Set in_place to rewrite the files instead of returning text.",
            )?),
            edited: None,
        })
    }
}
