//! Repository-local commands for building and running Weaver labs.

use std::env;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

const USAGE: &str = "\
Weaver lab tasks

Usage:
  cargo xtask run <lab> --platform desktop
  cargo xtask build <lab> --platform desktop|web
  cargo xtask check <lab> --platform desktop|web|all

Labs:
  first-person
  render
  space

Examples:
  cargo xtask run first-person --platform desktop
  cargo xtask build first-person --platform web
  cargo xtask check space --platform all
";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Action {
    Run,
    Build,
    Check,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Platform {
    Desktop,
    Web,
    All,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Lab {
    slug: &'static str,
    package: &'static str,
    wasm_stem: &'static str,
}

impl Lab {
    const FIRST_PERSON: Self = Self {
        slug: "first-person",
        package: "first-person-lab",
        wasm_stem: "first_person_lab",
    };
    const RENDER: Self = Self {
        slug: "render",
        package: "render-lab",
        wasm_stem: "render_lab",
    };
    const SPACE: Self = Self {
        slug: "space",
        package: "space-lab",
        wasm_stem: "space_lab",
    };

    fn parse(value: &str) -> Result<Self, String> {
        match value {
            "first-person" | "first-person-lab" => Ok(Self::FIRST_PERSON),
            "render" | "render-lab" => Ok(Self::RENDER),
            "space" | "space-lab" => Ok(Self::SPACE),
            _ => Err(format!("unknown lab `{value}`")),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Task {
    action: Action,
    lab: Lab,
    platform: Platform,
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}\n\n{USAGE}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let arguments: Vec<String> = env::args().skip(1).collect();
    if arguments
        .iter()
        .any(|argument| argument == "-h" || argument == "--help")
    {
        print!("{USAGE}");
        return Ok(());
    }
    let task = parse_task(&arguments)?;
    let root = workspace_root();

    match (task.action, task.platform) {
        (Action::Run, Platform::Desktop) => run_cargo(
            &root,
            ["run", "-p", task.lab.package, "--bin", task.lab.package],
        ),
        (Action::Run, Platform::Web) => Err(
            "the web shell is static output; use `build --platform web`, then host `dist/`"
                .to_owned(),
        ),
        (Action::Run, Platform::All) => Err("`run` does not support platform `all`".to_owned()),
        (Action::Build, Platform::Desktop) => run_cargo(&root, ["build", "-p", task.lab.package]),
        (Action::Build, Platform::Web) => build_web(&root, task.lab),
        (Action::Build, Platform::All) => Err("`build` does not support platform `all`".to_owned()),
        (Action::Check, Platform::Desktop) => check_desktop(&root, task.lab),
        (Action::Check, Platform::Web) => check_web(&root, task.lab),
        (Action::Check, Platform::All) => {
            check_desktop(&root, task.lab)?;
            check_web(&root, task.lab)
        }
    }
}

fn parse_task(arguments: &[String]) -> Result<Task, String> {
    if arguments.len() != 4 || arguments[2] != "--platform" {
        return Err("expected `<action> <lab> --platform <platform>`".to_owned());
    }
    let action = match arguments[0].as_str() {
        "run" => Action::Run,
        "build" => Action::Build,
        "check" => Action::Check,
        action => return Err(format!("unknown action `{action}`")),
    };
    let platform = match arguments[3].as_str() {
        "desktop" => Platform::Desktop,
        "web" => Platform::Web,
        "all" => Platform::All,
        platform => return Err(format!("unknown platform `{platform}`")),
    };
    Ok(Task {
        action,
        lab: Lab::parse(&arguments[1])?,
        platform,
    })
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask belongs directly under the workspace root")
        .to_owned()
}

fn check_desktop(root: &Path, lab: Lab) -> Result<(), String> {
    run_cargo(root, ["check", "-p", lab.package, "--all-targets"])
}

fn check_web(root: &Path, lab: Lab) -> Result<(), String> {
    run_cargo(
        root,
        [
            "check",
            "-p",
            lab.package,
            "--target",
            "wasm32-unknown-unknown",
            "--lib",
        ],
    )
}

fn build_web(root: &Path, lab: Lab) -> Result<(), String> {
    verify_wasm_bindgen(root)?;
    run_cargo(
        root,
        [
            "build",
            "--release",
            "-p",
            lab.package,
            "--target",
            "wasm32-unknown-unknown",
            "--lib",
        ],
    )?;

    let output = root.join("dist").join(lab.slug);
    if output.exists() {
        fs::remove_dir_all(&output).map_err(|error| {
            format!("failed to clear web output `{}`: {error}", output.display())
        })?;
    }
    fs::create_dir_all(output.join("pkg")).map_err(|error| {
        format!(
            "failed to create web output `{}`: {error}",
            output.display()
        )
    })?;
    let web_source = root
        .join("examples")
        .join(lab.package)
        .join("platforms/web");
    copy_directory(&web_source, &output)?;

    let wasm = root
        .join("target/wasm32-unknown-unknown/release")
        .join(format!("{}.wasm", lab.wasm_stem));
    run_command(
        root,
        "wasm-bindgen",
        [
            OsStr::new("--target"),
            OsStr::new("web"),
            OsStr::new("--no-typescript"),
            OsStr::new("--out-dir"),
            output.join("pkg").as_os_str(),
            wasm.as_os_str(),
        ],
    )?;
    if web_source.join("package.json").is_file() {
        let woven_client = root
            .parent()
            .expect("Weaver belongs to the Signalweave workspace")
            .join("woven/crates/woven-client-ts");
        if !woven_client.join("package.json").is_file() {
            return Err(format!(
                "the sibling Woven TypeScript client is missing at `{}`",
                woven_client.display()
            ));
        }
        run_command(
            &woven_client,
            "npm",
            ["ci", "--ignore-scripts", "--no-audit", "--no-fund"],
        )?;
        run_command(&woven_client, "npm", ["run", "build"])?;
        run_command(
            &web_source,
            "npm",
            ["ci", "--ignore-scripts", "--no-audit", "--no-fund"],
        )?;
        run_command(&web_source, "npm", ["run", "typecheck"])?;
        let output_argument = format!("--outfile={}", output.join("main.js").display());
        run_command(
            &web_source,
            "npm",
            ["run", "build", "--", output_argument.as_str()],
        )?;
    }

    println!("\nBuilt {} for web at {}", lab.package, output.display());
    println!("Host separately from the workspace root:");
    println!(
        "  python3 -m http.server 8000 --bind 127.0.0.1 --directory dist/{}",
        lab.slug
    );
    Ok(())
}

fn verify_wasm_bindgen(root: &Path) -> Result<(), String> {
    let required = locked_package_version(&root.join("Cargo.lock"), "wasm-bindgen")?;
    let output = Command::new("wasm-bindgen")
        .arg("--version")
        .current_dir(root)
        .output()
        .map_err(|error| {
            format!(
                "failed to run `wasm-bindgen --version`: {error}; install version {required} with `cargo install wasm-bindgen-cli --version {required} --locked`"
            )
        })?;
    if !output.status.success() {
        return Err("`wasm-bindgen --version` failed".to_owned());
    }
    let actual = String::from_utf8_lossy(&output.stdout);
    let actual = actual
        .split_whitespace()
        .last()
        .ok_or_else(|| "could not parse `wasm-bindgen --version` output".to_owned())?;
    if actual != required {
        return Err(format!(
            "wasm-bindgen CLI {actual} does not match Cargo.lock {required}; install the matching CLI with `cargo install wasm-bindgen-cli --version {required} --locked`"
        ));
    }
    Ok(())
}

fn locked_package_version(lockfile: &Path, package: &str) -> Result<String, String> {
    let contents = fs::read_to_string(lockfile)
        .map_err(|error| format!("failed to read `{}`: {error}", lockfile.display()))?;
    for section in contents.split("[[package]]").skip(1) {
        let mut name = None;
        let mut version = None;
        for line in section.lines() {
            if let Some(value) = line.strip_prefix("name = ") {
                name = quoted_value(value);
            } else if let Some(value) = line.strip_prefix("version = ") {
                version = quoted_value(value);
            }
        }
        if name.as_deref() == Some(package) {
            return version
                .ok_or_else(|| format!("package `{package}` has no version in Cargo.lock"));
        }
    }
    Err(format!("package `{package}` is absent from Cargo.lock"))
}

fn quoted_value(value: &str) -> Option<String> {
    value
        .strip_prefix('"')
        .and_then(|value| value.strip_suffix('"'))
        .map(ToOwned::to_owned)
}

fn copy_directory(source: &Path, destination: &Path) -> Result<(), String> {
    for entry in fs::read_dir(source)
        .map_err(|error| format!("failed to read `{}`: {error}", source.display()))?
    {
        let entry = entry.map_err(|error| format!("failed to read web asset entry: {error}"))?;
        let file_type = entry
            .file_type()
            .map_err(|error| format!("failed to inspect `{}`: {error}", entry.path().display()))?;
        if skip_web_build_input(&entry.file_name()) {
            continue;
        }
        let target = destination.join(entry.file_name());
        if file_type.is_dir() {
            fs::create_dir_all(&target)
                .map_err(|error| format!("failed to create `{}`: {error}", target.display()))?;
            copy_directory(&entry.path(), &target)?;
        } else if file_type.is_file() {
            fs::copy(entry.path(), &target)
                .map_err(|error| format!("failed to copy `{}`: {error}", target.display()))?;
        }
    }
    Ok(())
}

fn skip_web_build_input(name: &OsStr) -> bool {
    matches!(
        name.to_str(),
        Some(
            "node_modules"
                | "package.json"
                | "package-lock.json"
                | "tsconfig.json"
                | "main.ts"
                | "pkg"
                | "woven.local-token"
        )
    )
}

fn run_cargo<const N: usize>(root: &Path, arguments: [&str; N]) -> Result<(), String> {
    run_command(root, "cargo", arguments)
}

fn run_command<I, S>(root: &Path, program: &str, arguments: I) -> Result<(), String>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let status = Command::new(program)
        .args(arguments)
        .current_dir(root)
        .status()
        .map_err(|error| format!("failed to start `{program}`: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("`{program}` exited with {status}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(values: &[&str]) -> Vec<String> {
        values.iter().map(ToString::to_string).collect()
    }

    #[test]
    fn parses_supported_task() {
        assert_eq!(
            parse_task(&strings(&["check", "first-person", "--platform", "all"])),
            Ok(Task {
                action: Action::Check,
                lab: Lab::FIRST_PERSON,
                platform: Platform::All,
            })
        );
    }

    #[test]
    fn rejects_unknown_lab() {
        let error = parse_task(&strings(&["build", "unknown", "--platform", "web"]))
            .expect_err("unknown lab should be rejected");
        assert!(error.contains("unknown lab"));
    }

    #[test]
    fn finds_locked_package_version() {
        let path = workspace_root().join("Cargo.lock");
        assert!(locked_package_version(&path, "wasm-bindgen").is_ok());
    }
}
