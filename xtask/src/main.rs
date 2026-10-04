//! Repository-local commands for building and running Weaver labs.

use std::env;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::sync::atomic::{AtomicU64, Ordering};

const USAGE: &str = "\
Weaver lab tasks

Usage:
  cargo xtask run <lab> --platform desktop
  cargo xtask build <lab> --platform desktop|web
  cargo xtask check <lab> --platform desktop|web|all

Static artifact validation (no build):
  node scripts/check-web-artifact.mjs <artifact-directory> [first-person|render|space]

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

    let mut staging = WebStaging::new(root, lab)?;
    let output = staging.artifact();
    fs::create_dir(output.join("pkg"))
        .map_err(|error| format!("failed to create staged WASM directory: {error}"))?;
    let web_source = root
        .join("examples")
        .join(lab.package)
        .join("platforms/web");
    copy_runtime_sources(&web_source, &output, lab)?;
    copy_web_licenses(root, &output, lab)?;

    let wasm = root
        .join("target/wasm32-unknown-unknown/release")
        .join(format!("{}.wasm", lab.wasm_stem));
    require_regular_file(&wasm)?;
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
    if lab == Lab::FIRST_PERSON {
        let woven_client = root
            .parent()
            .expect("Weaver belongs to the Signalweave workspace")
            .join("woven/crates/woven-client-ts");
        require_regular_file(&woven_client.join("package.json"))?;
        require_regular_file(&web_source.join("package.json"))?;
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
            [
                "run",
                "build",
                "--",
                "--legal-comments=inline",
                output_argument.as_str(),
            ],
        )?;
        copy_dependency_licenses(&woven_client, &output)?;
    }

    let license_packager = root.join("scripts/web-licenses.mjs");
    require_regular_file(&license_packager)?;
    run_command(
        root,
        "node",
        [
            license_packager.as_os_str(),
            OsStr::new(lab.slug),
            output.as_os_str(),
        ],
    )?;
    validate_web_artifact(root, &output, lab)?;
    staging.publish()?;
    let output = root.join("dist").join(lab.slug);
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

fn copy_runtime_sources(source: &Path, destination: &Path, lab: Lab) -> Result<(), String> {
    // Render and Space currently load WASM from inline modules in index.html.
    // First-Person's main.js is bundled into staging, never copied from source.
    if ![Lab::FIRST_PERSON, Lab::RENDER, Lab::SPACE].contains(&lab) {
        return Err("no runtime source allowlist exists for this lab".to_owned());
    }
    for name in ["index.html", "styles.css"] {
        copy_regular_file(&source.join(name), &destination.join(name))?;
    }
    Ok(())
}

fn require_no_symlinks(path: &Path) -> Result<(), String> {
    let mut checked = PathBuf::new();
    for component in path.components() {
        checked.push(component.as_os_str());
        let metadata = fs::symlink_metadata(&checked)
            .map_err(|error| format!("failed to inspect `{}`: {error}", checked.display()))?;
        if metadata.file_type().is_symlink() {
            return Err(format!(
                "symlinks are not permitted: `{}`",
                checked.display()
            ));
        }
    }
    Ok(())
}

fn require_regular_file(path: &Path) -> Result<(), String> {
    require_no_symlinks(path)?;
    let metadata = fs::symlink_metadata(path)
        .map_err(|error| format!("failed to inspect `{}`: {error}", path.display()))?;
    if !metadata.is_file() || metadata.len() == 0 {
        return Err(format!(
            "expected a nonempty regular file: `{}`",
            path.display()
        ));
    }
    Ok(())
}

fn copy_regular_file(source: &Path, destination: &Path) -> Result<(), String> {
    require_regular_file(source)?;
    require_no_symlinks(
        destination
            .parent()
            .ok_or("copy destination has no parent")?,
    )?;
    match fs::symlink_metadata(destination) {
        Ok(_) => {
            return Err(format!(
                "copy destination already exists: `{}`",
                destination.display()
            ));
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(format!("failed to inspect copy destination: {error}")),
    }
    fs::copy(source, destination)
        .map_err(|error| format!("failed to copy `{}`: {error}", source.display()))?;
    Ok(())
}

fn copy_optional_notices(source: &Path, licenses: &Path, label: &str) -> Result<(), String> {
    require_no_symlinks(source)?;
    for (name, suffix) in [("NOTICE", "NOTICE"), ("NOTICE.txt", "NOTICE-text")] {
        let notice = source.join(name);
        match fs::symlink_metadata(&notice) {
            Ok(_) => copy_regular_file(&notice, &licenses.join(format!("{label}-{suffix}.txt")))?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(format!(
                    "failed to inspect notice `{}`: {error}",
                    notice.display()
                ));
            }
        }
    }
    Ok(())
}

fn copy_web_licenses(root: &Path, output: &Path, lab: Lab) -> Result<(), String> {
    let licenses = output.join("licenses");
    fs::create_dir(&licenses)
        .map_err(|error| format!("failed to create staged license directory: {error}"))?;
    copy_regular_file(&root.join("LICENSE"), &licenses.join("Weaver-LICENSE.txt"))?;
    copy_regular_file(
        &root.join("crates/weaver-render-wgpu/assets/fonts/OFL.txt"),
        &licenses.join("NotoSans-OFL.txt"),
    )?;
    copy_optional_notices(root, &licenses, "Weaver")?;
    if lab == Lab::FIRST_PERSON {
        let woven = root
            .parent()
            .ok_or("Weaver has no sibling directory")?
            .join("woven");
        copy_regular_file(&woven.join("LICENSE"), &licenses.join("Woven-LICENSE.txt"))?;
        copy_regular_file(
            &woven.join("crates/woven-client-ts/LICENSE"),
            &licenses.join("Woven-Client-LICENSE.txt"),
        )?;
        copy_optional_notices(&woven, &licenses, "Woven")?;
        copy_optional_notices(
            &woven.join("crates/woven-client-ts"),
            &licenses,
            "Woven-Client",
        )?;
    }
    Ok(())
}

fn copy_dependency_licenses(woven_client: &Path, output: &Path) -> Result<(), String> {
    // FlatBuffers is the Woven browser client's runtime dependency. Read its
    // installed license, not an inferred SPDX license or a downloaded template.
    let flatbuffers = woven_client.join("node_modules/flatbuffers");
    require_no_symlinks(&flatbuffers)?;
    let mut license = None;
    for name in ["LICENSE", "LICENSE.txt", "LICENCE", "LICENCE.txt"] {
        let candidate = flatbuffers.join(name);
        match fs::symlink_metadata(&candidate) {
            Ok(_) => {
                require_regular_file(&candidate)?;
                if license.is_none() {
                    license = Some(candidate);
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(format!("failed to inspect dependency license: {error}")),
        }
    }
    let license = license.ok_or("installed FlatBuffers package has no recognized LICENSE file")?;
    let licenses = output.join("licenses");
    copy_regular_file(&license, &licenses.join("FlatBuffers-LICENSE.txt"))?;
    copy_optional_notices(&flatbuffers, &licenses, "FlatBuffers")
}

fn validate_web_artifact(root: &Path, output: &Path, lab: Lab) -> Result<(), String> {
    let validator = root.join("scripts/check-web-artifact.mjs");
    require_regular_file(&validator)?;
    run_command(
        root,
        "node",
        [
            validator.as_os_str(),
            output.as_os_str(),
            OsStr::new(lab.slug),
        ],
    )
}

static STAGING_ID: AtomicU64 = AtomicU64::new(0);

struct WebStaging {
    directory: PathBuf,
    output: PathBuf,
    preserve_backup: bool,
}

impl WebStaging {
    fn new(root: &Path, lab: Lab) -> Result<Self, String> {
        require_no_symlinks(root)?;
        let dist = root.join("dist");
        match fs::create_dir(&dist) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(format!("failed to create dist directory: {error}")),
        }
        require_no_symlinks(&dist)?;
        let directory = loop {
            let id = STAGING_ID.fetch_add(1, Ordering::Relaxed);
            let candidate = dist.join(format!(".{}-build-{}-{id}", lab.slug, std::process::id()));
            match fs::create_dir(&candidate) {
                Ok(()) => break candidate,
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                Err(error) => {
                    return Err(format!("failed to create web staging directory: {error}"));
                }
            }
        };
        let staging = Self {
            directory,
            output: dist.join(lab.slug),
            preserve_backup: false,
        };
        fs::create_dir(staging.artifact())
            .map_err(|error| format!("failed to create staged artifact: {error}"))?;
        Ok(staging)
    }

    fn artifact(&self) -> PathBuf {
        self.directory.join("artifact")
    }

    fn publish(&mut self) -> Result<(), String> {
        self.publish_with(|source, destination| fs::rename(source, destination))
    }

    fn publish_with<F>(&mut self, mut rename: F) -> Result<(), String>
    where
        F: FnMut(&Path, &Path) -> std::io::Result<()>,
    {
        require_no_symlinks(&self.artifact())?;
        let previous = self.directory.join("previous");
        let had_previous = match fs::symlink_metadata(&self.output) {
            Ok(metadata) => {
                require_no_symlinks(&self.output)?;
                if !metadata.is_dir() {
                    return Err(format!(
                        "web output is not a directory: `{}`",
                        self.output.display()
                    ));
                }
                rename(&self.output, &previous).map_err(|error| {
                    format!("failed to preserve previous web artifact: {error}")
                })?;
                true
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
            Err(error) => return Err(format!("failed to inspect previous web artifact: {error}")),
        };
        if let Err(error) = rename(&self.artifact(), &self.output) {
            if had_previous && let Err(rollback) = rename(&previous, &self.output) {
                // Do not let Drop delete the only remaining successful build.
                self.preserve_backup = true;
                return Err(format!(
                    "failed to publish web artifact: {error}; rollback failed: {rollback}; previous artifact retained at `{}`",
                    previous.display()
                ));
            }
            return Err(format!("failed to publish web artifact: {error}"));
        }
        Ok(())
    }
}

impl Drop for WebStaging {
    fn drop(&mut self) {
        if !self.preserve_backup
            && let Err(error) = fs::remove_dir_all(&self.directory)
        {
            eprintln!(
                "warning: failed to remove web staging directory `{}`: {error}",
                self.directory.display()
            );
        }
    }
}

fn cargo_arguments<const N: usize>(arguments: [&str; N]) -> Vec<&str> {
    let mut locked = Vec::with_capacity(N + 1);
    if let Some((subcommand, rest)) = arguments.split_first() {
        locked.push(*subcommand);
        locked.push("--locked");
        locked.extend_from_slice(rest);
    }
    locked
}

fn run_cargo<const N: usize>(root: &Path, arguments: [&str; N]) -> Result<(), String> {
    run_command(root, "cargo", cargo_arguments(arguments))
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

    struct TempRoot(PathBuf);

    impl TempRoot {
        fn new() -> Self {
            loop {
                let id = STAGING_ID.fetch_add(1, Ordering::Relaxed);
                let path =
                    env::temp_dir().join(format!("weaver-xtask-test-{}-{id}", std::process::id()));
                match fs::create_dir(&path) {
                    Ok(()) => return Self(path),
                    Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
                    Err(error) => panic!("failed to create test directory: {error}"),
                }
            }
        }
    }

    impl Drop for TempRoot {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn write_fixture(root: &Path, name: &str, contents: impl AsRef<[u8]>) {
        let path = root.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, contents).unwrap();
    }

    fn artifact_fixture(root: &Path, lab: Lab) {
        let module = if lab == Lab::FIRST_PERSON {
            "<script type=\"module\" src=\"main.js\"></script>".to_owned()
        } else {
            format!(
                "<script type=\"module\">import init from './pkg/{}.js';</script>",
                lab.wasm_stem
            )
        };
        write_fixture(
            root,
            "index.html",
            format!(
                "<!doctype html><html><head><link rel=\"stylesheet\" href=\"styles.css\"></head><body>{module}</body></html>"
            ),
        );
        write_fixture(root, "styles.css", ":root { color: white; }\n");
        if lab == Lab::FIRST_PERSON {
            write_fixture(
                root,
                "main.js",
                "import init from './pkg/first_person_lab.js';\n",
            );
        }
        write_fixture(
            root,
            &format!("pkg/{}.js", lab.wasm_stem),
            format!(
                "const wasm = new URL('{}_bg.wasm', import.meta.url);\nexport default function init() {{ return wasm; }}\n",
                lab.wasm_stem
            ),
        );
        // A complete minimal module with one function and an exported memory.
        let wasm: &[u8] = &[
            0, 97, 115, 109, 1, 0, 0, 0, 1, 4, 1, 96, 0, 0, 3, 2, 1, 0, 5, 3, 1, 0, 1, 7, 17, 2, 6,
            b'm', b'e', b'm', b'o', b'r', b'y', 2, 0, 4, b'm', b'a', b'i', b'n', 0, 0, 10, 4, 1, 2,
            0, 11,
        ];
        write_fixture(root, &format!("pkg/{}_bg.wasm", lab.wasm_stem), wasm);
        let license = fs::read(workspace_root().join("LICENSE")).unwrap();
        write_fixture(root, "licenses/Weaver-LICENSE.txt", &license);
        write_fixture(
            root,
            "licenses/NotoSans-OFL.txt",
            fs::read(workspace_root().join("crates/weaver-render-wgpu/assets/fonts/OFL.txt"))
                .unwrap(),
        );
        let status = Command::new("node")
            .current_dir(workspace_root())
            .args([
                "--input-type=module",
                "-e",
                "import { noticeArtifacts } from './scripts/web-licenses.mjs'; import { writeFileSync } from 'node:fs'; import path from 'node:path'; for (const [name, bytes] of noticeArtifacts(process.argv[1])) writeFileSync(path.join(process.argv[2], name), bytes);",
                lab.slug,
            ])
            .arg(root)
            .status()
            .expect("license fixtures require Node.js");
        assert!(status.success());
        if lab == Lab::FIRST_PERSON {
            // Synthetic artifacts use the same complete Apache text as test data.
            for name in [
                "Woven-LICENSE.txt",
                "Woven-Client-LICENSE.txt",
                "FlatBuffers-LICENSE.txt",
            ] {
                write_fixture(root, &format!("licenses/{name}"), &license);
            }
        }
    }

    fn check_fixture(root: &Path, lab: Lab) -> Result<(), String> {
        let output = Command::new("node")
            .arg(workspace_root().join("scripts/check-web-artifact.mjs"))
            .arg(root)
            .arg(lab.slug)
            .output()
            .expect("artifact validator tests require Node.js");
        if output.status.success() {
            Ok(())
        } else {
            Err(String::from_utf8_lossy(&output.stderr).into_owned())
        }
    }

    #[test]
    fn cargo_subcommands_are_locked_before_runtime_arguments() {
        assert_eq!(
            cargo_arguments(["run", "-p", "first-person-lab", "--", "argument"]),
            [
                "run",
                "--locked",
                "-p",
                "first-person-lab",
                "--",
                "argument"
            ]
        );
        assert_eq!(cargo_arguments(["build"]), ["build", "--locked"]);
        assert_eq!(cargo_arguments(["check"]), ["check", "--locked"]);
    }

    #[test]
    fn copies_only_current_runtime_sources_for_all_three_labs() {
        let temp = TempRoot::new();
        let source = temp.0.join("source");
        for name in [
            "index.html",
            "styles.css",
            "main.js",
            "main.ts",
            "helper.ts",
            "helper.test.mjs",
            ".env",
            "woven.local-token",
        ] {
            write_fixture(&source, name, "synthetic fixture only");
        }
        write_fixture(&source, "pkg/stale.js", "stale generated input");
        write_fixture(&source, "node_modules/private.txt", "not a runtime input");
        for lab in [Lab::FIRST_PERSON, Lab::RENDER, Lab::SPACE] {
            let output = temp.0.join(lab.slug);
            fs::create_dir(&output).unwrap();
            copy_runtime_sources(&source, &output, lab).unwrap();
            let mut names: Vec<_> = fs::read_dir(output)
                .unwrap()
                .map(|entry| entry.unwrap().file_name())
                .collect();
            names.sort();
            assert_eq!(names, [OsStr::new("index.html"), OsStr::new("styles.css")]);
        }
    }

    #[test]
    fn missing_or_empty_runtime_input_fails_closed() {
        let temp = TempRoot::new();
        write_fixture(&temp.0, "source/index.html", "html");
        fs::create_dir(temp.0.join("output")).unwrap();
        assert!(
            copy_runtime_sources(&temp.0.join("source"), &temp.0.join("output"), Lab::RENDER)
                .is_err()
        );
        write_fixture(&temp.0, "source/styles.css", "");
        assert!(require_regular_file(&temp.0.join("source/styles.css")).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn rejects_selected_source_symlinks_and_symlinked_ancestors() {
        use std::os::unix::fs::symlink;
        let temp = TempRoot::new();
        write_fixture(&temp.0, "real/index.html", "html");
        write_fixture(&temp.0, "real/styles.css", "css");
        fs::create_dir(temp.0.join("source")).unwrap();
        fs::create_dir(temp.0.join("output")).unwrap();
        symlink(
            temp.0.join("real/index.html"),
            temp.0.join("source/index.html"),
        )
        .unwrap();
        symlink(temp.0.join("real"), temp.0.join("linked-source")).unwrap();
        assert!(
            copy_runtime_sources(&temp.0.join("source"), &temp.0.join("output"), Lab::RENDER)
                .unwrap_err()
                .contains("symlink")
        );
        assert!(
            copy_runtime_sources(
                &temp.0.join("linked-source"),
                &temp.0.join("output"),
                Lab::RENDER
            )
            .unwrap_err()
            .contains("symlink")
        );
    }

    #[test]
    fn copies_real_license_files_and_only_existing_notice_names() {
        let temp = TempRoot::new();
        write_fixture(
            &temp.0,
            "client/node_modules/flatbuffers/LICENSE.txt",
            "actual fixture license text",
        );
        write_fixture(
            &temp.0,
            "client/node_modules/flatbuffers/NOTICE",
            "actual fixture notice",
        );
        write_fixture(
            &temp.0,
            "client/node_modules/flatbuffers/NOTICE.txt",
            "second actual fixture notice",
        );
        write_fixture(
            &temp.0,
            "client/node_modules/flatbuffers/private-token",
            "not a license",
        );
        fs::create_dir_all(temp.0.join("artifact/licenses")).unwrap();
        copy_dependency_licenses(&temp.0.join("client"), &temp.0.join("artifact")).unwrap();
        assert_eq!(
            fs::read_to_string(temp.0.join("artifact/licenses/FlatBuffers-LICENSE.txt")).unwrap(),
            "actual fixture license text"
        );
        assert!(
            temp.0
                .join("artifact/licenses/FlatBuffers-NOTICE.txt")
                .is_file()
        );
        assert!(
            temp.0
                .join("artifact/licenses/FlatBuffers-NOTICE-text.txt")
                .is_file()
        );
        assert!(!temp.0.join("artifact/licenses/private-token").exists());
    }

    #[test]
    fn abandoned_staging_preserves_previous_output_and_cleans_partial_files() {
        let temp = TempRoot::new();
        write_fixture(
            &temp.0,
            "dist/first-person/previous.txt",
            "previous successful build",
        );
        let directory;
        {
            let staging = WebStaging::new(&temp.0, Lab::FIRST_PERSON).unwrap();
            directory = staging.directory.clone();
            write_fixture(&staging.artifact(), "partial.js", "incomplete");
        }
        assert!(!directory.exists());
        assert_eq!(
            fs::read_to_string(temp.0.join("dist/first-person/previous.txt")).unwrap(),
            "previous successful build"
        );
    }

    #[test]
    fn invalid_staged_artifact_never_replaces_previous_output() {
        let temp = TempRoot::new();
        write_fixture(
            &temp.0,
            "dist/first-person/previous.txt",
            "previous successful build",
        );
        {
            let staging = WebStaging::new(&temp.0, Lab::FIRST_PERSON).unwrap();
            artifact_fixture(&staging.artifact(), Lab::FIRST_PERSON);
            write_fixture(
                &staging.artifact(),
                "woven.local-token",
                "synthetic fixture only",
            );
            assert!(
                check_fixture(&staging.artifact(), Lab::FIRST_PERSON)
                    .unwrap_err()
                    .contains("unexpected artifact file")
            );
        }
        assert!(temp.0.join("dist/first-person/previous.txt").is_file());
    }

    #[test]
    fn validated_staging_replaces_only_selected_lab() {
        let temp = TempRoot::new();
        write_fixture(&temp.0, "dist/first-person/previous.txt", "old");
        write_fixture(&temp.0, "dist/render/untouched.txt", "other lab");
        let directory;
        {
            let mut staging = WebStaging::new(&temp.0, Lab::FIRST_PERSON).unwrap();
            directory = staging.directory.clone();
            artifact_fixture(&staging.artifact(), Lab::FIRST_PERSON);
            check_fixture(&staging.artifact(), Lab::FIRST_PERSON).unwrap();
            staging.publish().unwrap();
        }
        assert!(!directory.exists());
        assert!(!temp.0.join("dist/first-person/previous.txt").exists());
        check_fixture(&temp.0.join("dist/first-person"), Lab::FIRST_PERSON).unwrap();
        assert!(temp.0.join("dist/render/untouched.txt").is_file());
    }

    #[test]
    fn publish_error_rolls_back_previous_build() {
        let temp = TempRoot::new();
        write_fixture(&temp.0, "dist/first-person/previous.txt", "old");
        {
            let mut staging = WebStaging::new(&temp.0, Lab::FIRST_PERSON).unwrap();
            let artifact = staging.artifact();
            write_fixture(&artifact, "new.txt", "new");
            let result = staging.publish_with(|source, destination| {
                if source == artifact {
                    Err(std::io::Error::other("injected promotion failure"))
                } else {
                    fs::rename(source, destination)
                }
            });
            assert!(result.unwrap_err().contains("injected promotion failure"));
        }
        assert_eq!(
            fs::read_to_string(temp.0.join("dist/first-person/previous.txt")).unwrap(),
            "old"
        );
    }

    #[test]
    fn rollback_failure_retains_previous_build_for_recovery() {
        let temp = TempRoot::new();
        write_fixture(&temp.0, "dist/first-person/previous.txt", "old");
        let previous;
        {
            let mut staging = WebStaging::new(&temp.0, Lab::FIRST_PERSON).unwrap();
            previous = staging.directory.join("previous");
            let output = staging.output.clone();
            let result = staging.publish_with(|source, destination| {
                if destination == output {
                    Err(std::io::Error::other("injected publish/rollback failure"))
                } else {
                    fs::rename(source, destination)
                }
            });
            assert!(result.unwrap_err().contains("previous artifact retained"));
        }
        assert_eq!(
            fs::read_to_string(previous.join("previous.txt")).unwrap(),
            "old"
        );
    }

    #[test]
    fn reviewed_web_license_inventory_regressions() {
        let status = Command::new("node")
            .current_dir(workspace_root())
            .args(["--test", "scripts/web-licenses.test.mjs"])
            .status()
            .expect("license inventory tests require Node.js");
        assert!(status.success());
    }

    #[test]
    fn standalone_validator_accepts_each_current_lab_inventory() {
        for lab in [Lab::FIRST_PERSON, Lab::RENDER, Lab::SPACE] {
            let temp = TempRoot::new();
            artifact_fixture(&temp.0, lab);
            check_fixture(&temp.0, lab).unwrap();
        }
    }

    #[test]
    fn standalone_validator_rejects_sources_secrets_and_unexpected_files() {
        for name in [
            "woven.local-token",
            ".env",
            "connection-log.ts",
            "user-profile.test.mjs",
            "package.json",
            "pkg/first_person_lab.d.ts",
            "licenses/arbitrary.txt",
        ] {
            let temp = TempRoot::new();
            artifact_fixture(&temp.0, Lab::FIRST_PERSON);
            write_fixture(&temp.0, name, "synthetic fixture only");
            assert!(
                check_fixture(&temp.0, Lab::FIRST_PERSON)
                    .unwrap_err()
                    .contains("unexpected artifact file")
            );
        }
    }

    #[test]
    fn standalone_validator_rejects_missing_empty_and_truncated_artifacts() {
        let temp = TempRoot::new();
        artifact_fixture(&temp.0, Lab::FIRST_PERSON);
        fs::remove_file(temp.0.join("main.js")).unwrap();
        assert!(
            check_fixture(&temp.0, Lab::FIRST_PERSON)
                .unwrap_err()
                .contains("missing required artifact")
        );
        write_fixture(&temp.0, "main.js", "");
        assert!(
            check_fixture(&temp.0, Lab::FIRST_PERSON)
                .unwrap_err()
                .contains("nonempty regular file")
        );
        artifact_fixture(&temp.0, Lab::FIRST_PERSON);
        write_fixture(&temp.0, "main.js", "import {");
        assert!(
            check_fixture(&temp.0, Lab::FIRST_PERSON)
                .unwrap_err()
                .contains("invalid or truncated JavaScript")
        );
        artifact_fixture(&temp.0, Lab::FIRST_PERSON);
        let wasm = temp.0.join("pkg/first_person_lab_bg.wasm");
        let mut bytes = fs::read(&wasm).unwrap();
        bytes.pop();
        fs::write(wasm, bytes).unwrap();
        assert!(
            check_fixture(&temp.0, Lab::FIRST_PERSON)
                .unwrap_err()
                .contains("invalid or truncated WebAssembly")
        );
        artifact_fixture(&temp.0, Lab::FIRST_PERSON);
        write_fixture(&temp.0, "index.html", "<!doctype html><html><body>");
        assert!(
            check_fixture(&temp.0, Lab::FIRST_PERSON)
                .unwrap_err()
                .contains("incomplete HTML")
        );
        artifact_fixture(&temp.0, Lab::FIRST_PERSON);
        write_fixture(&temp.0, "styles.css", ":root { color: white;");
        assert!(
            check_fixture(&temp.0, Lab::FIRST_PERSON)
                .unwrap_err()
                .contains("incomplete stylesheet")
        );
    }

    #[test]
    fn standalone_validator_rejects_incomplete_styles_and_exportless_wasm() {
        let temp = TempRoot::new();
        for css in [
            "{ :root { color: white; }",
            ":root { color: 'white; }",
            ":root { color: white; } /* unfinished",
        ] {
            artifact_fixture(&temp.0, Lab::FIRST_PERSON);
            write_fixture(&temp.0, "styles.css", css);
            assert!(
                check_fixture(&temp.0, Lab::FIRST_PERSON)
                    .unwrap_err()
                    .contains("incomplete stylesheet")
            );
        }
        artifact_fixture(&temp.0, Lab::FIRST_PERSON);
        write_fixture(
            &temp.0,
            "styles.css",
            ":root { color: white; } /* complete comment */",
        );
        check_fixture(&temp.0, Lab::FIRST_PERSON).unwrap();
        write_fixture(
            &temp.0,
            "pkg/first_person_lab_bg.wasm",
            [0, 97, 115, 109, 1, 0, 0, 0],
        );
        assert!(
            check_fixture(&temp.0, Lab::FIRST_PERSON)
                .unwrap_err()
                .contains("lacks runtime exports")
        );
    }

    #[test]
    fn standalone_validator_checks_module_and_wasm_cross_references() {
        let temp = TempRoot::new();
        artifact_fixture(&temp.0, Lab::FIRST_PERSON);
        write_fixture(&temp.0, "main.js", "import init from './pkg/missing.js';");
        assert!(
            check_fixture(&temp.0, Lab::FIRST_PERSON)
                .unwrap_err()
                .contains("missing imported module")
        );
        artifact_fixture(&temp.0, Lab::FIRST_PERSON);
        write_fixture(
            &temp.0,
            "pkg/first_person_lab.js",
            "export default function init() {};",
        );
        assert!(
            check_fixture(&temp.0, Lab::FIRST_PERSON)
                .unwrap_err()
                .contains("does not reference")
        );
    }

    #[test]
    fn standalone_validator_allows_only_referenced_generated_snippets() {
        let temp = TempRoot::new();
        artifact_fixture(&temp.0, Lab::FIRST_PERSON);
        write_fixture(
            &temp.0,
            "pkg/snippets/winit-fixture/runtime.js",
            "export const value = 1;",
        );
        assert!(
            check_fixture(&temp.0, Lab::FIRST_PERSON)
                .unwrap_err()
                .contains("unreferenced runtime module")
        );
        let glue = temp.0.join("pkg/first_person_lab.js");
        let source = fs::read_to_string(&glue).unwrap();
        fs::write(
            glue,
            format!("import './snippets/winit-fixture/runtime.js';\n{source}"),
        )
        .unwrap();
        check_fixture(&temp.0, Lab::FIRST_PERSON).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn standalone_validator_rejects_file_directory_and_root_symlinks() {
        use std::os::unix::fs::symlink;
        let temp = TempRoot::new();
        let artifact = temp.0.join("artifact");
        artifact_fixture(&artifact, Lab::FIRST_PERSON);
        fs::rename(artifact.join("styles.css"), temp.0.join("real.css")).unwrap();
        symlink(temp.0.join("real.css"), artifact.join("styles.css")).unwrap();
        assert!(
            check_fixture(&artifact, Lab::FIRST_PERSON)
                .unwrap_err()
                .contains("symlinks are forbidden")
        );
        fs::remove_file(artifact.join("styles.css")).unwrap();
        write_fixture(&artifact, "styles.css", ":root { color: white; }");
        fs::rename(artifact.join("pkg"), temp.0.join("real-pkg")).unwrap();
        symlink(temp.0.join("real-pkg"), artifact.join("pkg")).unwrap();
        assert!(
            check_fixture(&artifact, Lab::FIRST_PERSON)
                .unwrap_err()
                .contains("symlinks are forbidden")
        );
        symlink(&artifact, temp.0.join("linked-artifact")).unwrap();
        assert!(
            check_fixture(&temp.0.join("linked-artifact"), Lab::FIRST_PERSON)
                .unwrap_err()
                .contains("symlinks are forbidden")
        );
    }
}
