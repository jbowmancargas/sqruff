use crate::commands::FixArgs;
use crate::commands::Format;
use crate::linter;
use sqruff_lib::core::config::FluffConfig;
use sqruff_lib::core::linter::core::Linter;
use std::io::{self, Write};
use std::path::Path;

/// Extra fix passes over already-fixed SQL. Some fixes change the structure
/// (e.g. TQ02 wraps a body in BEGIN/END) and the rest of the file only lines
/// up with that once it is parsed again.
const EXTRA_FIX_PASSES: usize = 2;

/// Re-lint and fix `fixed` until it stops changing (or the pass limit is hit).
fn refix(linter: &Linter, mut fixed: String, fix_even_unparsable: bool) -> String {
    for _ in 0..EXTRA_FIX_PASSES {
        let Ok(result) = linter.lint_string(&fixed, None, true) else {
            break;
        };
        if !result.has_fixes() || (result.has_parse_or_templating_errors() && !fix_even_unparsable)
        {
            break;
        }
        let again = result.fix_file_string();
        if again == fixed {
            break;
        }
        fixed = again;
    }
    fixed
}

/// A linter for the extra passes that does not report violations again.
fn quiet_linter(config: &FluffConfig, collect_parse_errors: bool) -> Option<Linter> {
    linter(
        config.clone(),
        Format::None,
        collect_parse_errors,
        true,
        true,
    )
    .ok()
}

pub(crate) fn run_fix(
    args: FixArgs,
    config: FluffConfig,
    ignorer: impl Fn(&Path) -> bool + Send + Sync,
    collect_parse_errors: bool,
) -> i32 {
    let FixArgs {
        paths,
        format,
        quiet,
        verbose: _,
        disregard_sqruffignores,
    } = args;
    let fix_even_unparsable = config
        .get("fix_even_unparsable", "core")
        .as_bool()
        .unwrap_or(false);
    let large_file_skip_fail = config
        .get("large_file_skip_fail", "core")
        .as_bool()
        .unwrap_or(false);
    let refix_linter = quiet_linter(&config, collect_parse_errors);
    let mut linter = match linter(config, format, collect_parse_errors, quiet, true) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("{}", e);
            return 1;
        }
    };
    let result = match linter.lint_paths_with_ignore_files(
        paths,
        true,
        &ignorer,
        !disregard_sqruffignores,
    ) {
        Ok(result) => result,
        Err(e) => {
            eprintln!("{}", e.value);
            return 1;
        }
    };

    let skipped_files_should_fail = large_file_skip_fail && result.files_skipped() > 0;
    if !result.has_violations() && !result.has_fixable_violations() {
        match format {
            Format::Human if !quiet => {
                println!("{} files processed, nothing to fix.", result.len())
            }
            Format::Json | Format::Sarif | Format::GithubAnnotationNative => {
                linter.formatter().unwrap().completion_message(result.len());
            }
            _ => {}
        }
        skipped_files_should_fail as i32
    } else {
        let any_unfixable_errors = result.has_unfixable_violations();
        let files = result.len();

        for mut file in result {
            if !file.has_fixes() || (file.has_parse_or_templating_errors() && !fix_even_unparsable)
            {
                continue;
            }
            let path = std::mem::take(&mut file.path);
            let mut fixed = file.fix_file_string();
            if let Some(refix_linter) = &refix_linter {
                fixed = refix(refix_linter, fixed, fix_even_unparsable);
            }
            std::fs::write(path, fixed).unwrap();
        }

        linter.formatter_mut().unwrap().completion_message(files);

        (any_unfixable_errors || skipped_files_should_fail) as i32
    }
}

pub(crate) fn run_fix_stdin(
    config: FluffConfig,
    format: Format,
    quiet: bool,
    stdin_filename: Option<&Path>,
    ignorer: &(dyn Fn(&Path) -> bool + Send + Sync),
    disregard_ignores: bool,
    collect_parse_errors: bool,
) -> i32 {
    let read_in = crate::stdin::read_std_in().unwrap();
    let fix_even_unparsable = config
        .get("fix_even_unparsable", "core")
        .as_bool()
        .unwrap_or(false);

    let refix_linter = quiet_linter(&config, collect_parse_errors);
    let linter = match linter(config, format, collect_parse_errors, quiet, true) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("{}", e);
            return 1;
        }
    };
    if crate::stdin::stdin_filename_is_ignored(stdin_filename, !disregard_ignores, ignorer) {
        let mut stdout = io::stdout().lock();
        stdout.write_all(read_in.as_bytes()).unwrap();
        stdout.flush().unwrap();
        return 0;
    }
    let result = match linter.lint_string(&read_in, None, true) {
        Ok(result) => result,
        Err(e) => {
            eprintln!("{}", e.value);
            return 1;
        }
    };

    let has_unfixable_errors = result.has_unfixable_violations();

    let has_fixes = result.has_fixable_violations()
        && result.has_fixes()
        && (fix_even_unparsable || !result.has_parse_or_templating_errors());
    let output = if has_fixes {
        let fixed = result.fix_file_string();
        match &refix_linter {
            Some(refix_linter) => refix(refix_linter, fixed, fix_even_unparsable),
            None => fixed,
        }
    } else {
        read_in
    };
    let mut stdout = io::stdout().lock();
    stdout.write_all(output.as_bytes()).unwrap();
    stdout.flush().unwrap();

    // if all fixable violations are fixable, return 0 else return 1
    has_unfixable_errors as i32
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::path::Path;
    use std::thread::sleep;
    use std::time::Duration;

    fn ignore_none(_: &Path) -> bool {
        false
    }

    #[test]
    fn run_fix_does_not_update_mtime_when_no_changes() {
        let mut tmp = tempfile::Builder::new().suffix(".sql").tempfile().unwrap();
        writeln!(tmp, "SELECT 1 FROM").unwrap();
        tmp.flush().unwrap();
        let tmp = tmp.into_temp_path();
        let path = tmp.to_path_buf();

        let before = std::fs::metadata(&path).unwrap().modified().unwrap();
        sleep(Duration::from_secs(1));

        let args = FixArgs {
            paths: vec![path.clone()],
            format: Format::Human,
            quiet: false,
            verbose: 0,
            disregard_sqruffignores: false,
        };
        let config = FluffConfig::default();
        assert_eq!(run_fix(args, config, ignore_none, true), 1);

        let after = std::fs::metadata(&path).unwrap().modified().unwrap();
        assert_eq!(before, after);
    }

    #[test]
    fn run_fix_applies_warning_only_fixes() {
        let mut tmp = tempfile::Builder::new().suffix(".sql").tempfile().unwrap();
        write!(tmp, "     SELECT 1").unwrap();
        tmp.flush().unwrap();

        let args = FixArgs {
            paths: vec![tmp.path().to_path_buf()],
            format: Format::None,
            quiet: false,
            verbose: 0,
            disregard_sqruffignores: false,
        };
        let config = FluffConfig::from_source(
            "[sqruff]\ndialect = ansi\nrules = LT02\nwarnings = LT02\n",
            None,
        );

        assert_eq!(run_fix(args, config, ignore_none, true), 0);
        assert_eq!(std::fs::read_to_string(tmp.path()).unwrap(), "SELECT 1");
    }

    #[test]
    fn run_fix_applies_warning_and_error_fixes() {
        let mut tmp = tempfile::Builder::new().suffix(".sql").tempfile().unwrap();
        write!(tmp, "     SELECT a from foo").unwrap();
        tmp.flush().unwrap();

        let args = FixArgs {
            paths: vec![tmp.path().to_path_buf()],
            format: Format::None,
            quiet: false,
            verbose: 0,
            disregard_sqruffignores: false,
        };
        let config = FluffConfig::from_source(
            "[sqruff]\ndialect = ansi\nrules = LT02,CP01\nwarnings = LT02\n",
            None,
        );

        assert_eq!(run_fix(args, config, ignore_none, true), 0);
        assert_eq!(
            std::fs::read_to_string(tmp.path()).unwrap(),
            "SELECT a FROM foo"
        );
    }

    #[test]
    fn run_fix_reindents_after_structural_fixes() {
        let mut tmp = tempfile::Builder::new().suffix(".sql").tempfile().unwrap();
        write!(tmp, "CREATE PROCEDURE p\nAS\nSELECT 1;\nSELECT 2;\n").unwrap();
        tmp.flush().unwrap();

        let args = FixArgs {
            paths: vec![tmp.path().to_path_buf()],
            format: Format::None,
            quiet: false,
            verbose: 0,
            disregard_sqruffignores: false,
        };
        let config =
            FluffConfig::from_source("[sqruff]\ndialect = tsql\nrules = TQ02,LT02\n", None);

        run_fix(args, config, ignore_none, true);
        assert_eq!(
            std::fs::read_to_string(tmp.path()).unwrap(),
            "CREATE PROCEDURE p\nAS\nBEGIN\n    SELECT 1;\n    SELECT 2;\nEND\n"
        );
    }
}
