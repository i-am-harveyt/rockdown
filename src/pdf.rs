//! PDF export uses user-installed Pandoc >= 3.1.2 and Typst >= 0.13.
//! Markdown is data: only the trusted style below can introduce Typst code.

use anyhow::{Context, Result, bail, ensure};
use image::ImageDecoder;
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    env,
    fs::{self, File},
    io::{BufReader, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::atomic::{AtomicBool, Ordering},
    thread,
    time::Duration,
};

pub struct ExportRequest {
    pub source: String,
    pub source_path: Option<PathBuf>,
    pub base_dir: PathBuf,
    pub destination: PathBuf,
    pub config: crate::config::PdfConfig,
}

#[derive(Debug)]
pub struct ExportResult {
    pub warnings: String,
}

// Included *after* Pandoc's standard document wrapper so its defaults cannot
// override the fixed light A4 style. No source text is interpolated here.
const STYLE: &str = r#"
#set page(paper: "a4", margin: 20mm, fill: white)
#set text(fill: black, size: 11pt)
#set par(justify: false)
#show figure.where(kind: table): set block(breakable: true)
#show raw.where(block: true): block.with(
  width: 100%, breakable: true, fill: luma(96%), inset: 8pt, radius: 3pt,
)
#show image: it => layout(size => {
  let natural = measure(it, width: size.width)
  let factor = calc.min(1, size.width / natural.width, 237mm / natural.height)
  scale(x: factor * 100%, y: factor * 100%, reflow: true, it)
})
"#;

pub fn export(request: ExportRequest, cancelled: &AtomicBool) -> Result<ExportResult> {
    check_cancelled(cancelled)?;
    request.config.validate()?;
    let destination = destination_path(&request.destination, request.source_path.as_deref())?;
    // Resolve against the caller's PATH/cwd, never against the staging directory.
    let pandoc = resolve_tool(&request.config.pandoc, "pandoc")?;
    let typst = resolve_tool(&request.config.typst, "typst")?;
    let staging = tempfile::Builder::new().prefix("rockdown-pdf-").tempdir()?;
    let root = staging.path();
    let data_dir = root.join("pandoc-data");
    fs::create_dir(&data_dir)?;
    let mut warnings = String::new();

    tool_version(&pandoc, "Pandoc", [3, 1, 2], root, cancelled, &mut warnings)?;
    tool_version(&typst, "Typst", [0, 13, 0], root, cancelled, &mut warnings)?;
    let fonts = run(
        Command::new(&typst).arg("fonts"),
        "Listing Typst fonts",
        root,
        None,
        cancelled,
        &mut warnings,
    )?;
    let fonts = String::from_utf8(fs::read(fonts)?).context("Typst returned invalid font names")?;
    let style = font_style(&request.config, &fonts, &mut warnings)? + STYLE;
    let style_path = root.join("style.typ");
    fs::write(&style_path, style)?;

    let source_path = root.join("source.md");
    fs::write(&source_path, request.source)?;
    let ast_path = run(
        Command::new(&pandoc)
            .arg("--sandbox")
            .arg("--data-dir")
            .arg(&data_dir)
            .args(["--from=gfm-raw_attribute", "--to=json"]),
        "Reading Markdown with Pandoc",
        root,
        Some(&source_path),
        cancelled,
        &mut warnings,
    )?;
    let mut ast: Value = serde_json::from_reader(BufReader::new(File::open(ast_path)?))
        .context("Pandoc returned invalid document JSON")?;
    ensure!(
        ast.get("blocks").is_some_and(Value::is_array),
        "Pandoc returned no document blocks"
    );
    // Never accept document-supplied templates, includes, bibliography, or code.
    ast["meta"] = json!({});
    let mut assets = Assets {
        base: &request.base_dir,
        root,
        staged: HashMap::new(),
        cancelled,
    };
    adapt(&mut ast["blocks"], &mut assets)?;
    let json_path = root.join("document.json");
    serde_json::to_writer(File::create(&json_path)?, &ast)?;
    let generated = run(
        Command::new(&pandoc)
            .arg("--sandbox")
            .arg("--data-dir")
            .arg(&data_dir)
            .args(["--from=json", "--to=typst", "--standalone"])
            .arg("--include-before-body")
            .arg(&style_path),
        "Writing Typst with Pandoc",
        root,
        Some(&json_path),
        cancelled,
        &mut warnings,
    )?;
    let typst_path = root.join("document.typ");
    fs::rename(generated, &typst_path)?;
    let pdf_path = root.join("document.pdf");
    run(
        Command::new(&typst)
            .args(["compile", "--root"])
            .arg(root)
            .arg(&typst_path)
            .arg(&pdf_path),
        "Compiling PDF with Typst",
        root,
        None,
        cancelled,
        &mut warnings,
    )?;
    publish(
        &pdf_path,
        &destination,
        request.source_path.as_deref(),
        cancelled,
    )?;
    Ok(ExportResult { warnings })
}

fn check_cancelled(cancelled: &AtomicBool) -> Result<()> {
    ensure!(!cancelled.load(Ordering::Acquire), "PDF export cancelled");
    Ok(())
}

struct RunningChild(Child);

impl Drop for RunningChild {
    fn drop(&mut self) {
        // Also covers I/O failures and unwinding while polling a live child.
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

// All three streams are files, not pipes: large documents/diagnostics cannot
// deadlock on pipe capacity. The child is reaped on every exit path.
fn run(
    command: &mut Command,
    label: &str,
    root: &Path,
    input: Option<&Path>,
    cancelled: &AtomicBool,
    warnings: &mut String,
) -> Result<PathBuf> {
    check_cancelled(cancelled)?;
    let output = tempfile::NamedTempFile::new_in(root)?;
    let errors = tempfile::NamedTempFile::new_in(root)?;
    command
        .current_dir(root)
        .stdin(match input {
            Some(path) => Stdio::from(File::open(path)?),
            None => Stdio::null(),
        })
        .stdout(Stdio::from(output.reopen()?))
        .stderr(Stdio::from(errors.reopen()?));
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW in the GUI application.
    }
    let mut child = RunningChild(
        command
            .spawn()
            .with_context(|| format!("{label}: could not start tool"))?,
    );
    let status = loop {
        check_cancelled(cancelled)?;
        if let Some(status) = child
            .0
            .try_wait()
            .with_context(|| format!("{label}: waiting for tool"))?
        {
            break status;
        }
        thread::sleep(Duration::from_millis(30));
    };
    check_cancelled(cancelled)?;
    let diagnostics = String::from_utf8_lossy(&fs::read(errors.path())?)
        .trim()
        .to_owned();
    ensure!(
        status.success(),
        "{label} failed ({status}){}",
        if diagnostics.is_empty() {
            String::new()
        } else {
            format!(":\n{diagnostics}")
        }
    );
    if !diagnostics.is_empty() {
        if !warnings.is_empty() {
            warnings.push('\n');
        }
        warnings.push_str(label);
        warnings.push_str(":\n");
        warnings.push_str(&diagnostics);
    }
    // The enclosing TempDir owns retained outputs, including on later failure.
    let (_, path) = output.keep().context("Retaining tool output")?;
    Ok(path)
}

fn resolve_tool(configured: &str, name: &str) -> Result<PathBuf> {
    let configured_path = Path::new(configured);
    let candidates = if configured_path.is_absolute() {
        vec![configured_path.to_path_buf()]
    } else {
        let search_path = env::var_os("PATH");
        let directories = search_path
            .as_deref()
            .into_iter()
            .flat_map(env::split_paths);
        #[cfg(target_os = "macos")]
        let directories = directories.chain([
            PathBuf::from("/opt/homebrew/bin"),
            PathBuf::from("/usr/local/bin"),
        ]);
        #[cfg(windows)]
        let candidates = directories
            .flat_map(|directory| {
                let path = directory.join(configured);
                let exe = path.with_extension("exe");
                if path.extension().is_none() {
                    vec![path, exe]
                } else {
                    vec![path]
                }
            })
            .collect();
        #[cfg(not(windows))]
        let candidates = directories
            .map(|directory| directory.join(configured))
            .collect();
        candidates
    };
    for candidate in candidates {
        let Ok(metadata) = fs::metadata(&candidate) else {
            continue;
        };
        if !metadata.is_file() {
            continue;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if metadata.permissions().mode() & 0o111 == 0 {
                continue;
            }
        }
        if let Ok(path) = fs::canonicalize(candidate) {
            return Ok(path);
        }
    }
    bail!(
        "Cannot find executable {configured:?}. Install Pandoc >= 3.1.2 (https://pandoc.org/installing.html) and Typst >= 0.13 (https://github.com/typst/typst/releases); on macOS: brew install pandoc typst. Set pdf.{name} to its absolute executable path if it is not on PATH."
    )
}

fn tool_version(
    executable: &Path,
    name: &str,
    minimum: [u32; 3],
    root: &Path,
    cancelled: &AtomicBool,
    warnings: &mut String,
) -> Result<[u32; 3]> {
    let output = run(
        Command::new(executable).arg("--version"),
        &format!("Checking {name}"),
        root,
        None,
        cancelled,
        warnings,
    )?;
    let text = fs::read_to_string(output)?;
    let word = text
        .split_whitespace()
        .nth(1)
        .context("Tool did not report a version")?;
    let mut version = [0; 3];
    for (part, value) in word.split('.').take(3).zip(&mut version) {
        let digits = part.trim_end_matches(|c: char| !c.is_ascii_digit());
        *value = digits
            .parse()
            .with_context(|| format!("Unrecognized {name} version: {word}"))?;
    }
    ensure!(
        version >= minimum,
        "{name} {word} is too old for PDF export; install {name} >= {}.{}.{}",
        minimum[0],
        minimum[1],
        minimum[2]
    );
    Ok(version)
}

fn font_style(
    config: &crate::config::PdfConfig,
    available: &str,
    warnings: &mut String,
) -> Result<String> {
    let installed: Vec<_> = available.lines().map(str::trim).collect();
    let find = |name: &str| {
        installed
            .iter()
            .copied()
            .find(|font| font.eq_ignore_ascii_case(name))
    };
    let requested = |name: &Option<String>, field: &str| -> Result<Option<String>> {
        name.as_ref().map(|name| find(name).map(str::to_owned).with_context(||
            format!("Font {name:?} configured in pdf.{field} is not installed. Install it or choose a family listed by typst fonts.")))
            .transpose()
    };
    let cjk = [
        "PingFang TC",
        "PingFang SC",
        "Microsoft JhengHei",
        "Microsoft YaHei",
        "Noto Sans CJK TC",
        "Noto Sans CJK SC",
        "Noto Sans CJK JP",
        "Noto Sans CJK KR",
        "Source Han Sans TC",
        "Source Han Sans SC",
        "Hiragino Sans",
        "Yu Gothic",
        "Malgun Gothic",
    ]
    .into_iter()
    .find_map(find);
    let main = requested(&config.main_font, "main_font")?.or_else(|| cjk.map(str::to_owned));
    let mono = requested(&config.mono_font, "mono_font")?.or_else(|| {
        ["Menlo", "Consolas", "DejaVu Sans Mono", "Liberation Mono"]
            .into_iter()
            .find_map(find)
            .map(str::to_owned)
    });
    if cjk.is_none() && config.main_font.is_none() {
        if !warnings.is_empty() {
            warnings.push('\n');
        }
        warnings.push_str("No known CJK font found. For Chinese/Japanese/Korean text, install Noto Sans CJK or set pdf.main_font to an installed family; inspect the PDF for missing glyphs.");
    }
    // JSON quotes/backslashes have the same escaping in Typst strings. Config
    // validation excludes controls (whose JSON Unicode escapes differ).
    let mut style = String::new();
    if let Some(main) = &main {
        style.push_str(&format!(
            "#set text(font: {})\n",
            serde_json::to_string(main)?
        ));
    }
    if let Some(mono) = mono {
        let mut fonts = vec![mono];
        if let Some(cjk) = cjk.or(main.as_deref()) {
            fonts.push(cjk.to_owned());
        }
        let quoted: Vec<_> = fonts
            .iter()
            .map(serde_json::to_string)
            .collect::<std::result::Result<_, _>>()?;
        style.push_str(&format!(
            "#show raw: set text(font: ({},), size: 9pt)\n",
            quoted.join(", ")
        ));
    }
    Ok(style)
}

struct Assets<'a> {
    base: &'a Path,
    root: &'a Path,
    staged: HashMap<PathBuf, String>,
    cancelled: &'a AtomicBool,
}

impl Assets<'_> {
    fn stage(&mut self, url: &str) -> Result<String> {
        check_cancelled(self.cancelled)?;
        let path = crate::image_assets::local_image_path(self.base, url)
            .with_context(|| format!("Cannot export image {url:?}: only local PNG, JPEG, GIF, and WebP images are supported; remote, data, and network URLs are not fetched"))?;
        let path = fs::canonicalize(&path).with_context(|| format!("Cannot read image {url:?}"))?;
        #[cfg(windows)]
        if matches!(path.components().next(), Some(std::path::Component::Prefix(prefix)) if matches!(prefix.kind(), std::path::Prefix::UNC(..) | std::path::Prefix::VerbatimUNC(..)))
        {
            bail!("Cannot export network image {url:?}");
        }
        if let Some(name) = self.staged.get(&path) {
            return Ok(name.clone());
        }
        ensure!(
            fs::metadata(&path)?.is_file(),
            "Image {url:?} is not a regular file"
        );
        let reader = image::ImageReader::open(&path)?.with_guessed_format()?;
        ensure!(
            matches!(
                reader.format(),
                Some(
                    image::ImageFormat::Png
                        | image::ImageFormat::Jpeg
                        | image::ImageFormat::Gif
                        | image::ImageFormat::WebP
                )
            ),
            "Unsupported image {url:?}; use PNG, JPEG, GIF, or WebP (SVG is not passed to Typst)"
        );
        let mut decoder = reader
            .into_decoder()
            .with_context(|| format!("Invalid image {url:?}"))?;
        let mut limits = image::Limits::default();
        limits.reserve(decoder.total_bytes())?;
        decoder.set_limits(limits)?;
        let orientation = decoder.orientation()?;
        // DynamicImage decodes the first GIF frame; animation is not exported.
        let mut image = image::DynamicImage::from_decoder(decoder)
            .with_context(|| format!("Invalid image {url:?}"))?;
        image.apply_orientation(orientation);
        check_cancelled(self.cancelled)?;
        let name = format!("image-{:04}.png", self.staged.len() + 1);
        image.save_with_format(self.root.join(&name), image::ImageFormat::Png)?;
        self.staged.insert(path, name.clone());
        Ok(name)
    }
}

fn underline_tag(value: &Value) -> Option<&str> {
    if value["t"] == "RawInline" && value["c"][0] == "html" {
        match value["c"][1].as_str() {
            Some(tag @ ("<u>" | "</u>")) => Some(tag),
            _ => None,
        }
    } else {
        None
    }
}

fn underline_list(values: &mut Vec<Value>) {
    if !values.iter().any(|value| underline_tag(value).is_some()) {
        return;
    }
    let mut frames = vec![Vec::with_capacity(values.len())];
    for value in std::mem::take(values) {
        match underline_tag(&value) {
            Some("<u>") => frames.push(Vec::new()),
            Some("</u>") if frames.len() > 1 => {
                let contents = frames.pop().unwrap();
                frames
                    .last_mut()
                    .unwrap()
                    .push(json!({"t": "Underline", "c": contents}));
            }
            _ => frames.last_mut().unwrap().push(value),
        }
    }
    // An unmatched opener remains literal, while balanced nested spans survive.
    while frames.len() > 1 {
        let contents = frames.pop().unwrap();
        let parent = frames.last_mut().unwrap();
        parent.push(json!({"t": "Str", "c": "<u>"}));
        parent.extend(contents);
    }
    *values = frames.pop().unwrap();
}

fn image_width(title: &str) -> Option<String> {
    let title = title.trim();
    let (number, percent) = title
        .strip_suffix('%')
        .map(|n| (n, true))
        .or_else(|| title.strip_suffix("px").map(|n| (n, false)))?;
    let value: f32 = number.trim().parse().ok()?;
    (value.is_finite() && value > 0.0).then(|| {
        if percent {
            // Resolve against A4's 170 mm text area before Typst's image
            // show rule measures it; a relative width there is applied twice.
            format!("{}mm", value.min(100.0) * 1.7)
        } else {
            format!("{value}px")
        }
    })
}

fn adapt(value: &mut Value, assets: &mut Assets<'_>) -> Result<()> {
    check_cancelled(assets.cancelled)?;
    if let Some(values) = value.as_array_mut() {
        underline_list(values);
        for value in values {
            adapt(value, assets)?;
        }
        return Ok(());
    }
    match value["t"].as_str() {
        Some("RawInline" | "RawBlock") => {
            let text = value["c"][1]
                .as_str()
                .context("Invalid raw Pandoc node")?
                .to_owned();
            *value = if value["t"] == "RawBlock" {
                json!({"t": "CodeBlock", "c": [["", [], []], text]})
            } else if value["c"][0] == "html" {
                json!({"t": "Str", "c": text})
            } else {
                json!({"t": "Code", "c": [["", [], []], text]})
            };
            return Ok(());
        }
        Some("Image") => {
            let url = value["c"][2][0]
                .as_str()
                .context("Invalid Pandoc image target")?;
            let name = assets.stage(url)?;
            value["c"][2][0] = Value::String(name);
            let title = value["c"][2][1].as_str().unwrap_or_default();
            if let Some(width) = image_width(title) {
                value["c"][0][2] = json!([["width", width]]);
                value["c"][2][1] = json!("");
            }
        }
        _ => {}
    }
    if let Some(contents) = value.get_mut("c") {
        adapt(contents, assets)?;
    }
    Ok(())
}

fn destination_path(destination: &Path, source: Option<&Path>) -> Result<PathBuf> {
    ensure!(
        destination
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("pdf")),
        "PDF export destination must have a .pdf extension"
    );
    let parent = destination
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let destination = fs::canonicalize(parent)
        .context("PDF destination directory is unavailable")?
        .join(
            destination
                .file_name()
                .context("PDF destination needs a filename")?,
        );
    if let Some(source) = source {
        ensure!(
            !same_file(source, &destination)?,
            "PDF export cannot overwrite the source document or an alias of it"
        );
    }
    if let Ok(metadata) = fs::symlink_metadata(&destination) {
        ensure!(
            !metadata.file_type().is_symlink(),
            "PDF destination is a symbolic link; choose a regular PDF file"
        );
        ensure!(metadata.is_file(), "PDF destination is not a regular file");
    }
    Ok(destination)
}

fn same_file(left: &Path, right: &Path) -> Result<bool> {
    if left == right {
        return Ok(true);
    }
    let left_canonical = fs::canonicalize(left).ok();
    let right_canonical = fs::canonicalize(right).ok();
    if left_canonical.is_some() && left_canonical == right_canonical {
        return Ok(true);
    }
    let (Ok(left), Ok(right)) = (File::open(left), File::open(right)) else {
        return Ok(false);
    };
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let left = left.metadata()?;
        let right = right.metadata()?;
        Ok(left.dev() == right.dev() && left.ino() == right.ino())
    }
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::Storage::FileSystem::{
            BY_HANDLE_FILE_INFORMATION, GetFileInformationByHandle,
        };
        let info = |file: &File| -> Result<BY_HANDLE_FILE_INFORMATION> {
            let mut info = std::mem::MaybeUninit::uninit();
            ensure!(
                unsafe { GetFileInformationByHandle(file.as_raw_handle(), info.as_mut_ptr()) } != 0,
                "Cannot inspect file identity: {}",
                std::io::Error::last_os_error()
            );
            Ok(unsafe { info.assume_init() })
        };
        let left = info(&left)?;
        let right = info(&right)?;
        Ok(left.dwVolumeSerialNumber == right.dwVolumeSerialNumber
            && left.nFileIndexHigh == right.nFileIndexHigh
            && left.nFileIndexLow == right.nFileIndexLow)
    }
}

fn publish(
    pdf: &Path,
    destination: &Path,
    source: Option<&Path>,
    cancelled: &AtomicBool,
) -> Result<()> {
    check_cancelled(cancelled)?;
    let mut input = File::open(pdf).context("Typst did not create a PDF")?;
    let mut magic = [0; 5];
    input
        .read_exact(&mut magic)
        .context("Typst produced an empty or incomplete PDF")?;
    ensure!(&magic == b"%PDF-", "Typst did not produce a PDF file");
    input.seek(SeekFrom::Start(0))?;
    let mut temporary = tempfile::NamedTempFile::new_in(
        destination
            .parent()
            .context("Missing PDF destination directory")?,
    )?;
    std::io::copy(&mut input, temporary.as_file_mut())?;
    temporary.as_file_mut().flush()?;
    temporary.as_file().sync_all()?;
    // Recheck aliases after compilation; neither failure nor cancellation ever
    // truncates an existing destination. Persist is an atomic same-dir replace.
    destination_path(destination, source)?;
    check_cancelled(cancelled)?;
    temporary
        .persist(destination)
        .with_context(|| format!("Cannot replace PDF {}", destination.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn adapt_blocks(blocks: Value, base: &Path, root: &Path) -> Result<Value> {
        let mut blocks = blocks;
        let cancelled = AtomicBool::new(false);
        adapt(
            &mut blocks,
            &mut Assets {
                base,
                root,
                staged: HashMap::new(),
                cancelled: &cancelled,
            },
        )?;
        Ok(blocks)
    }

    #[test]
    fn underline_is_balanced_within_each_inline_list_and_raw_text_stays_literal() {
        let temp = tempfile::tempdir().unwrap();
        let raw = |text| json!({"t":"RawInline","c":["html",text]});
        let blocks = json!([
            {"t":"Para","c":[raw("<u>"), {"t":"Strong","c":[{"t":"Str","c":"世界"}]}, raw("</u>"), raw("<b>"), {"t":"Code","c":[["",[],[]],"</u>"]}]},
            {"t":"Para","c":[raw("<u>"), {"t":"Str","c":"unclosed"}]},
            {"t":"Para","c":[raw("</u>")]},
            {"t":"RawBlock","c":["typst","#read(\"/secret\")"]}
        ]);
        let output = adapt_blocks(blocks, temp.path(), temp.path()).unwrap();
        assert_eq!(
            output[0]["c"][0],
            json!({"t":"Underline","c":[{"t":"Strong","c":[{"t":"Str","c":"世界"}]}]})
        );
        assert_eq!(output[0]["c"][1], json!({"t":"Str","c":"<b>"}));
        assert_eq!(output[0]["c"][2]["t"], "Code");
        assert_eq!(output[1]["c"][0], json!({"t":"Str","c":"<u>"}));
        assert_eq!(output[2]["c"][0], json!({"t":"Str","c":"</u>"}));
        assert_eq!(
            output[3],
            json!({"t":"CodeBlock","c":[["",[],[]],"#read(\"/secret\")"]})
        );
    }

    #[test]
    fn images_are_local_decoded_snapshots_and_reject_unsafe_sources() {
        let temp = tempfile::tempdir().unwrap();
        let stage = tempfile::tempdir().unwrap();
        let source = temp.path().join("圖片 space.png");
        image::DynamicImage::new_rgb8(2, 3).save(&source).unwrap();
        let image =
            |url: &str| json!([{"t":"Para","c":[{"t":"Image","c":[["",[],[]],[],[url,"40%"]]}]}]);
        let output = adapt_blocks(
            image("%E5%9C%96%E7%89%87%20space.png"),
            temp.path(),
            stage.path(),
        )
        .unwrap();
        fs::remove_file(source).unwrap();
        let target = output[0]["c"][0]["c"][2][0].as_str().unwrap();
        let decoded = image::open(stage.path().join(target)).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (2, 3));
        for url in [
            "https://example.org/image.png",
            "data:image/png;base64,AAAA",
            "//server/photo.png",
            "missing.png",
        ] {
            assert!(
                adapt_blocks(image(url), temp.path(), stage.path()).is_err(),
                "{url}"
            );
        }
        fs::write(
            temp.path().join("unsafe.svg"),
            "<svg xmlns=\"http://www.w3.org/2000/svg\"/>",
        )
        .unwrap();
        assert!(adapt_blocks(image("unsafe.svg"), temp.path(), stage.path()).is_err());
    }

    #[test]
    fn failed_or_cancelled_publication_preserves_previous_pdf_and_source_aliases() {
        let temp = tempfile::tempdir().unwrap();
        let destination = temp.path().join("export.pdf");
        let staged = temp.path().join("staged.pdf");
        fs::write(&destination, b"previous PDF").unwrap();
        fs::write(&staged, b"not a PDF").unwrap();
        assert!(publish(&staged, &destination, None, &AtomicBool::new(false)).is_err());
        assert_eq!(fs::read(&destination).unwrap(), b"previous PDF");
        fs::write(&staged, b"%PDF-1.7\nnew PDF").unwrap();
        assert!(publish(&staged, &destination, None, &AtomicBool::new(true)).is_err());
        assert_eq!(fs::read(&destination).unwrap(), b"previous PDF");
        let source = temp.path().join("source.md");
        fs::write(&source, b"# Markdown").unwrap();
        let alias = temp.path().join("alias.pdf");
        fs::hard_link(&source, &alias).unwrap();
        assert!(publish(&staged, &alias, Some(&source), &AtomicBool::new(false)).is_err());
        assert_eq!(fs::read(&source).unwrap(), b"# Markdown");
        assert!(destination_path(&source, None).is_err());
        publish(&staged, &destination, None, &AtomicBool::new(false)).unwrap();
        assert_eq!(fs::read(&destination).unwrap(), b"%PDF-1.7\nnew PDF");
    }
}
