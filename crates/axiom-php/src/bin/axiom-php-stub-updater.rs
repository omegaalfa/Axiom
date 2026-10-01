use std::{
    env,
    error::Error,
    ffi::OsString,
    fs::{self, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    process::Command,
    time::{Instant, SystemTime, UNIX_EPOCH},
};

use axiom_php::{
    stub_generation::{generate_runtime_stub_artifact_binary, generate_runtime_stub_artifact_bytes, parse_phpstorm_stubs_map},
    EmbeddedStubArtifact, SymbolKind,
};
use serde::{Deserialize, Serialize};

const REQUIRED_SYMBOLS: [&str; 9] = [
    "DateTime",
    "DateTimeImmutable",
    "SplQueue",
    "ArrayIterator",
    "ReflectionClass",
    "PDO",
    "array_map",
    "array_filter",
    "strlen",
];

#[derive(Debug, Deserialize)]
struct StubLock {
    repository: String,
    commit: String,
    archive_sha256: String,
    map_path: String,
    artifact_schema_version: u32,
    symbol_parser_version: u32,
    generator_version: u32,
}

#[derive(Debug, Serialize)]
struct DatasetReport {
    source: String,
    source_mode: String,
    map: Option<String>,
    map_unique_files: usize,
    unreferenced_php_files: usize,
    repository: String,
    commit: String,
    files_processed: usize,
    symbols: usize,
    duplicate_symbols_removed: usize,
    classes: usize,
    interfaces: usize,
    traits: usize,
    enums: usize,
    methods: usize,
    properties: usize,
    class_constants: usize,
    functions: usize,
    global_constants: usize,
    runtime_stubs_binary_bytes: u64,
    elapsed_seconds: f64,
    required_symbols: Vec<String>,
    required_symbols_valid: bool,
}

fn main() -> Result<(), Box<dyn Error>> {
    let arguments: Vec<String> = env::args().skip(1).collect();
    match arguments.first().map(String::as_str) {
        None | Some("update") => update_bundled_dataset(),
        Some("generate") => generate_from_local_source(&arguments),
        Some("--source") => generate_from_external_source(&arguments),
        Some("--help") | Some("-h") => {
            println!(
                "axiom-php-stub-updater [update]\n\
                 axiom-php-stub-updater --source <PATH>\n\
                 axiom-php-stub-updater generate --root <PATH> --map <PATH> --output <PATH>"
            );
            Ok(())
        }
        Some(command) => Err(format!("unknown command {command}; run with --help").into()),
    }
}

fn update_bundled_dataset() -> Result<(), Box<dyn Error>> {
    let started = Instant::now();
    let crate_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let lock_path = crate_root.join("phpstorm-stubs.lock");
    let output_path = crate_root.join("embedded/runtime-stubs.bin");
    let lock = load_lock(&lock_path)?;

    let temporary = TemporaryDirectory::new()?;
    let archive_path = temporary.path().join("phpstorm-stubs.tar.gz");
    let source_root = temporary.path().join("source");
    let generated_path = temporary.path().join("runtime-stubs.bin");
    fs::create_dir_all(&source_root).map_err(|error| {
        io_context("create extraction directory", None, Some(&source_root), error)
    })?;

    let archive_url =
        format!("https://github.com/{}/archive/{}.tar.gz", lock.repository, lock.commit);
    download_archive(&archive_url, &archive_path)?;
    let actual_sha256 = file_sha256(&archive_path)?;
    if !actual_sha256.eq_ignore_ascii_case(&lock.archive_sha256) {
        return Err(format!(
            "SHA-256 mismatch for {}: expected {}, got {}",
            archive_path.display(),
            lock.archive_sha256,
            actual_sha256
        )
        .into());
    }

    run_command(
        "tar",
        [
            OsString::from("-xzf"),
            archive_path.as_os_str().to_owned(),
            OsString::from("-C"),
            source_root.as_os_str().to_owned(),
        ],
        &format!(
            "extract phpstorm-stubs archive source={} destination={}",
            archive_path.display(),
            source_root.display()
        ),
    )?;
    let (archive_root, map_path) = resolve_archive_root(&source_root, &lock.map_path)?;
    let report = generate_runtime_stub_artifact_binary(&archive_root, &map_path, &generated_path)?;
    let artifact_bytes = fs::read(&generated_path).map_err(|error| {
        io_context("read generated artifact", Some(&generated_path), None, error)
    })?;
    let artifact: EmbeddedStubArtifact = bincode::deserialize(&artifact_bytes)?;
    validate_required_symbols(&artifact)?;
    let counts = count_symbols(&artifact);
    install_atomically(&generated_path, &output_path)?;

    let dataset_report = DatasetReport {
        source: archive_root.display().to_string(),
        source_mode: "upstream-map".into(),
        map: Some(map_path.display().to_string()),
        map_unique_files: report.files_processed,
        unreferenced_php_files: 0,
        repository: lock.repository.clone(),
        commit: lock.commit.clone(),
        files_processed: report.files_processed,
        symbols: report.symbols_generated,
        duplicate_symbols_removed: report.duplicate_symbols_removed,
        classes: counts.classes,
        interfaces: counts.interfaces,
        traits: counts.traits,
        enums: counts.enums,
        methods: counts.methods,
        properties: counts.properties,
        class_constants: counts.class_constants,
        functions: counts.functions,
        global_constants: counts.global_constants,
        runtime_stubs_binary_bytes: fs::metadata(&output_path)
            .map_err(|error| io_context("read installed artifact metadata", Some(&output_path), None, error))?
            .len(),
        elapsed_seconds: started.elapsed().as_secs_f64(),
        required_symbols: REQUIRED_SYMBOLS.iter().map(|name| (*name).to_owned()).collect(),
        required_symbols_valid: true,
    };
    println!("{}", serde_json::to_string_pretty(&dataset_report)?);
    Ok(())
}

fn generate_from_external_source(arguments: &[String]) -> Result<(), Box<dyn Error>> {
    let started = Instant::now();
    let crate_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let source = PathBuf::from(required_value(arguments, "--source")?);
    if !source.is_dir() {
        return Err(format!("local stub source is not a directory: {}", source.display()).into());
    }
    let map_path = source.join("AxiomStubsMap.php");
    if !map_path.is_file() {
        return Err(format!("local stub source is missing canonical map: {}", map_path.display()).into());
    }
    let map_content = fs::read_to_string(&map_path).map_err(|error| {
        io_context("read local AxiomStubsMap.php", Some(&map_path), None, error)
    })?;
    let map = parse_phpstorm_stubs_map(&map_content)?;
    let map_files = map
        .classes
        .values()
        .chain(map.functions.values())
        .chain(map.constants.values())
        .map(PathBuf::from)
        .collect::<std::collections::BTreeSet<_>>();
    if map_files.is_empty() {
        return Err("local AxiomStubsMap.php references no PHP files".into());
    }
    for relative in &map_files {
        let path = source.join(relative);
        if !path.is_file() {
            return Err(format!("local AxiomStubsMap.php references missing PHP file: {}", path.display()).into());
        }
    }
    let files = enumerate_local_php_files(&source)?;
    let unreferenced_php_files = files.iter().filter(|file| !map_files.contains(*file)).count();
    let temporary = TemporaryDirectory::new()?;
    let generated_path = temporary.path().join("runtime-stubs.bin");
    let (artifact, report) = generate_runtime_stub_artifact_bytes(&source, map)?;
    let bytes = bincode::serialize(&artifact)?;
    fs::write(&generated_path, &bytes).map_err(|error| {
        io_context("write locally generated artifact", None, Some(&generated_path), error)
    })?;
    let artifact: EmbeddedStubArtifact = bincode::deserialize(
        &fs::read(&generated_path).map_err(|error| {
            io_context("read locally generated artifact", Some(&generated_path), None, error)
        })?,
    )?;
    validate_required_symbols(&artifact)?;
    let output_path = crate_root.join("embedded/runtime-stubs.bin");
    install_atomically(&generated_path, &output_path)?;
    let counts = count_symbols(&artifact);
    println!(
        "{}",
        serde_json::to_string_pretty(&DatasetReport {
            source: source.display().to_string(),
            source_mode: "axiom-map".into(),
            map: Some(map_path.display().to_string()),
            map_unique_files: map_files.len(),
            unreferenced_php_files,
            repository: "local".into(),
            commit: "local".into(),
            files_processed: report.files_processed,
            symbols: report.symbols_generated,
            duplicate_symbols_removed: report.duplicate_symbols_removed,
            classes: counts.classes,
            interfaces: counts.interfaces,
            traits: counts.traits,
            enums: counts.enums,
            methods: counts.methods,
            properties: counts.properties,
            class_constants: counts.class_constants,
            functions: counts.functions,
            global_constants: counts.global_constants,
            runtime_stubs_binary_bytes: fs::metadata(&output_path)?.len(),
            elapsed_seconds: started.elapsed().as_secs_f64(),
            required_symbols: REQUIRED_SYMBOLS.iter().map(|name| (*name).to_owned()).collect(),
            required_symbols_valid: true,
        })?
    );
    Ok(())
}

fn enumerate_local_php_files(root: &Path) -> Result<Vec<PathBuf>, Box<dyn Error>> {
    let mut files = Vec::new();
    collect_local_php_files(root, root, &mut files)?;
    files.sort();
    files.dedup();
    Ok(files)
}

fn collect_local_php_files(root: &Path, directory: &Path, files: &mut Vec<PathBuf>) -> Result<(), Box<dyn Error>> {
    let mut entries = fs::read_dir(directory)
        .map_err(|error| io_context("enumerate local stub directory", Some(directory), None, error))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| io_context("read local stub directory entry", Some(directory), None, error))?;
    entries.sort_by_key(|entry| entry.file_name());
    for entry in entries {
        let path = entry.path();
        let relative = path.strip_prefix(root).expect("local path is under root");
        if relative.components().any(|component| matches!(component, std::path::Component::Normal(name) if matches!(name.to_str(), Some(".git" | "target" | "build" | "cache")))) {
            continue;
        }
        if path.is_dir() {
            collect_local_php_files(root, &path, files)?;
        } else if path.extension().and_then(|extension| extension.to_str()).is_some_and(|extension| extension.eq_ignore_ascii_case("php"))
            && path.file_name().and_then(|name| name.to_str()) != Some("PhpStormStubsMap.php")
        {
            files.push(relative.to_path_buf());
        }
    }
    Ok(())
}

fn generate_from_local_source(arguments: &[String]) -> Result<(), Box<dyn Error>> {
    let root = required_value(arguments, "--root")?;
    let map = required_value(arguments, "--map")?;
    let output = required_value(arguments, "--output")?;
    let report = generate_runtime_stub_artifact_binary(
        &PathBuf::from(root),
        &PathBuf::from(map),
        &PathBuf::from(output),
    )?;
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}

fn load_lock(path: &Path) -> Result<StubLock, Box<dyn Error>> {
    let content = fs::read_to_string(path).map_err(|error| {
        io_context("read stub lock", Some(path), None, error)
    })?;
    let lock: StubLock = serde_json::from_str(&content)?;
    if !valid_repository(&lock.repository) {
        return Err(format!("invalid repository in {}: {}", path.display(), lock.repository).into());
    }
    if !lock.commit.chars().all(|character| character.is_ascii_hexdigit()) || lock.commit.len() != 40 {
        return Err(format!("invalid commit in {}: {}", path.display(), lock.commit).into());
    }
    if lock.archive_sha256.len() != 64
        || !lock
            .archive_sha256
            .chars()
            .all(|character| character.is_ascii_hexdigit())
    {
        return Err(format!("invalid archive SHA-256 in {}", path.display()).into());
    }
    if lock.artifact_schema_version != 1
        || lock.symbol_parser_version != 3
        || lock.generator_version != 1
    {
        return Err(format!("unsupported generator metadata in {}", path.display()).into());
    }
    Ok(lock)
}

fn valid_repository(repository: &str) -> bool {
    let Some((owner, name)) = repository.split_once('/') else {
        return false;
    };
    [owner, name].iter().all(|part| {
        !part.is_empty()
            && part
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || matches!(character, '-' | '_' | '.'))
    })
}

fn download_archive(url: &str, destination: &Path) -> Result<(), Box<dyn Error>> {
    run_command(
        "curl",
        [
            OsString::from("--fail"),
            OsString::from("--location"),
            OsString::from("--silent"),
            OsString::from("--show-error"),
            OsString::from("--retry"),
            OsString::from("2"),
            OsString::from("--output"),
            destination.as_os_str().to_owned(),
            OsString::from(url),
        ],
        "download pinned phpstorm-stubs archive",
    )
    .map_err(|error| format!("download archive destination={}: {error}", destination.display()).into())
}

fn file_sha256(path: &Path) -> Result<String, Box<dyn Error>> {
    #[cfg(windows)]
    {
        let output = run_command_capture(
            "certutil",
            [
                OsString::from("-hashfile"),
                path.as_os_str().to_owned(),
                OsString::from("SHA256"),
            ],
            &format!("calculate SHA-256 source={}", path.display()),
        )?;
        output
            .lines()
            .map(str::trim)
            .find(|line| {
                line.chars()
                    .all(|character| character.is_ascii_hexdigit() || character.is_whitespace())
                    && line.chars().any(|character| character.is_ascii_hexdigit())
            })
            .map(|line| line.chars().filter(|character| !character.is_whitespace()).collect())
            .ok_or_else(|| -> Box<dyn Error> { "certutil returned no SHA-256 hash".into() })
    }
    #[cfg(not(windows))]
    {
        let output = run_command_capture(
            "sha256sum",
            [path.as_os_str().to_owned()],
            &format!("calculate SHA-256 source={}", path.display()),
        )?;
        output
            .split_whitespace()
            .next()
            .map(str::to_owned)
            .ok_or_else(|| -> Box<dyn Error> { "sha256sum returned no hash".into() })
    }
}

fn resolve_archive_root(
    extracted_root: &Path,
    map_file_name: &str,
) -> Result<(PathBuf, PathBuf), Box<dyn Error>> {
    let mut map_paths = Vec::new();
    find_file_recursive(extracted_root, map_file_name, &mut map_paths)?;

    let mut roots = map_paths
        .iter()
        .filter_map(|path| path.parent().map(Path::to_path_buf))
        .collect::<Vec<_>>();
    roots.sort();
    roots.dedup();

    match roots.len() {
        0 => Err(format!(
            "could not resolve phpstorm-stubs archive root: no {map_file_name} found under {}",
            extracted_root.display()
        )
        .into()),
        1 if map_paths.len() == 1 => {
            let root = roots.pop().expect("one root candidate exists");
            Ok((root, map_paths.pop().expect("one map path exists")))
        }
        1 => Err(format!(
            "could not resolve phpstorm-stubs archive root: multiple {map_file_name} files found under {}",
            roots[0].display()
        )
        .into()),
        count => Err(format!(
            "could not resolve phpstorm-stubs archive root: found {count} candidates for {map_file_name} under {}",
            extracted_root.display()
        )
        .into()),
    }
}

fn find_file_recursive(
    directory: &Path,
    file_name: &str,
    matches: &mut Vec<PathBuf>,
) -> io::Result<()> {
    for entry in fs::read_dir(directory)? {
        let path = entry?.path();
        if path.is_dir() {
            find_file_recursive(&path, file_name, matches)?;
        } else if path.file_name().and_then(|name| name.to_str()) == Some(file_name) {
            matches.push(path);
        }
    }
    Ok(())
}

fn validate_required_symbols(artifact: &EmbeddedStubArtifact) -> Result<(), Box<dyn Error>> {
    let names: Vec<&str> = artifact
        .symbols
        .iter()
        .map(|symbol| symbol.fqn.trim_start_matches('\\'))
        .collect();
    let missing: Vec<&str> = REQUIRED_SYMBOLS
        .iter()
        .copied()
        .filter(|required| !names.contains(required))
        .collect();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(format!("generated artifact is missing required symbols: {}", missing.join(", ")).into())
    }
}

#[derive(Default)]
struct SymbolCounts {
    classes: usize,
    interfaces: usize,
    traits: usize,
    enums: usize,
    methods: usize,
    properties: usize,
    class_constants: usize,
    functions: usize,
    global_constants: usize,
}

fn count_symbols(artifact: &EmbeddedStubArtifact) -> SymbolCounts {
    let mut counts = SymbolCounts::default();
    for symbol in &artifact.symbols {
        match symbol.kind {
            SymbolKind::Class => counts.classes += 1,
            SymbolKind::Interface => counts.interfaces += 1,
            SymbolKind::Trait => counts.traits += 1,
            SymbolKind::Enum => counts.enums += 1,
            SymbolKind::Method => counts.methods += 1,
            SymbolKind::Property => counts.properties += 1,
            SymbolKind::ClassConstant => counts.class_constants += 1,
            SymbolKind::Function => counts.functions += 1,
            SymbolKind::GlobalConstant => counts.global_constants += 1,
        }
    }
    counts
}

fn install_atomically(source: &Path, target: &Path) -> Result<(), Box<dyn Error>> {
    let bytes = fs::read(source).map_err(|error| {
        io_context("read generated artifact for atomic install", Some(source), None, error)
    })?;
    let temporary = target.with_file_name(format!(
        ".{}.tmp-{}",
        target.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("runtime-stubs.bin"),
        std::process::id()
    ));
    let backup = target.with_file_name(format!(
        ".{}.backup-{}",
        target.file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("runtime-stubs.bin"),
        std::process::id()
    ));
    {
        let mut file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&temporary)
            .map_err(|error| {
                io_context("create atomic-install temporary file", None, Some(&temporary), error)
            })?;
        file.write_all(&bytes).map_err(|error| {
            io_context("write atomic-install temporary file", None, Some(&temporary), error)
        })?;
        file.flush().map_err(|error| {
            io_context("flush atomic-install temporary file", Some(&temporary), None, error)
        })?;
        file.sync_all().map_err(|error| {
            io_context("sync atomic-install temporary file", Some(&temporary), None, error)
        })?;
    }
    if target.exists() {
        fs::rename(target, &backup).map_err(|error| {
            io_context("move existing artifact to atomic-install backup", Some(target), Some(&backup), error)
        })?;
        if let Err(error) = fs::rename(&temporary, target) {
            let _ = fs::rename(&backup, target);
            return Err(io_context("atomically replace embedded runtime stubs", Some(&temporary), Some(target), error));
        }
        fs::remove_file(&backup).map_err(|error| {
            io_context("remove atomic-install backup", Some(&backup), None, error)
        })?;
    } else {
        fs::rename(&temporary, target).map_err(|error| {
            io_context("atomically install embedded runtime stubs", Some(&temporary), Some(target), error)
        })?;
    }
    Ok(())
}

fn io_context(
    operation: &str,
    source: Option<&Path>,
    destination: Option<&Path>,
    error: io::Error,
) -> Box<dyn Error> {
    format!(
        "{operation} source={} destination={} cause={error}",
        source.map_or("<none>".into(), |path| path.display().to_string()),
        destination.map_or("<none>".into(), |path| path.display().to_string()),
    )
    .into()
}

fn run_command<I, S>(program: &str, arguments: I, action: &str) -> Result<(), Box<dyn Error>>
where
    I: IntoIterator<Item = S>,
    S: Into<OsString>,
{
    run_command_capture(program, arguments, action).map(|_| ())
}

fn run_command_capture<I, S>(program: &str, arguments: I, action: &str) -> Result<String, Box<dyn Error>>
where
    I: IntoIterator<Item = S>,
    S: Into<OsString>,
{
    let output = Command::new(program)
        .args(arguments.into_iter().map(Into::into))
        .output()
        .map_err(|error| -> Box<dyn Error> {
            format!("{action} requires {program}: {error}").into()
        })?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stdout = String::from_utf8_lossy(&output.stdout);
        return Err(format!("{action} failed: {stderr}{stdout}").into());
    }
    Ok(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn required_value(arguments: &[String], name: &str) -> Result<String, Box<dyn Error>> {
    let position = arguments
        .iter()
        .position(|argument| argument == name)
        .ok_or_else(|| -> Box<dyn Error> { format!("missing {name}").into() })?;
    arguments
        .get(position + 1)
        .cloned()
        .ok_or_else(|| -> Box<dyn Error> { format!("missing value for {name}").into() })
}

#[cfg(test)]
mod tests {
    use super::{required_value, resolve_archive_root};
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn resolves_the_archive_subdirectory_containing_the_map() {
        let source = tempdir().unwrap();
        let archive_root = source.path().join("phpstorm-stubs-arbitrary-root");
        fs::create_dir_all(archive_root.join("Core")).unwrap();
        fs::write(archive_root.join("PhpStormStubsMap.php"), "map").unwrap();
        fs::write(archive_root.join("Core/Core.php"), "<?php").unwrap();

        let (resolved_root, map_path) =
            resolve_archive_root(source.path(), "PhpStormStubsMap.php").unwrap();

        assert_eq!(resolved_root, archive_root);
        assert_eq!(map_path, archive_root.join("PhpStormStubsMap.php"));
    }

    #[test]
    fn rejects_multiple_archive_root_candidates() {
        let source = tempdir().unwrap();
        for name in ["phpstorm-stubs-one", "phpstorm-stubs-two"] {
            let root = source.path().join(name);
            fs::create_dir_all(root.join("Core")).unwrap();
            fs::write(root.join("PhpStormStubsMap.php"), "map").unwrap();
            fs::write(root.join("Core/Core.php"), "<?php").unwrap();
        }

        let error = resolve_archive_root(source.path(), "PhpStormStubsMap.php")
            .unwrap_err()
            .to_string();
        assert!(error.contains("multiple") || error.contains("candidates"));
    }

    #[test]
    fn source_cli_requires_exact_path_value() {
        let arguments = vec!["--source".to_owned(), "E:/dev/axiom-stubs".to_owned()];
        assert_eq!(required_value(&arguments, "--source").unwrap(), "E:/dev/axiom-stubs");
        assert!(required_value(&["--source".to_owned()], "--source").is_err());
        assert!(required_value(&["--other".to_owned(), "value".to_owned()], "--source").is_err());
    }
}

struct TemporaryDirectory(PathBuf);

impl TemporaryDirectory {
    fn new() -> Result<Self, Box<dyn Error>> {
        let timestamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let path = env::temp_dir().join(format!("axiom-php-stubs-{}-{timestamp}", std::process::id()));
        fs::create_dir_all(&path).map_err(|error| {
            io_context("create temporary directory", None, Some(&path), error)
        })?;
        Ok(Self(path))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TemporaryDirectory {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}
