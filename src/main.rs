use anyhow::{anyhow, bail, Context, Result};
use biblatex::*;
use clap::{Parser, Subcommand, ValueEnum};
use serde::Deserialize;
use skim::prelude::*;
use std::{fs::File, io::BufReader, path::PathBuf};
use std::{fs::OpenOptions, io::prelude::*};

const URLS: [&str; 2] = ["https://dblp.org", "https://dblp.uni-trier.de"];

#[derive(Deserialize, Debug)]
struct DblpResponse {
    result: DblpResult,
}

impl DblpResponse {
    fn matches(&self) -> impl Iterator<Item = DblpHitInfo> + '_ {
        self.result.hits.hit.iter().map(|hit| hit.info.clone())
    }
}

#[derive(Deserialize, Debug)]
struct DblpResult {
    hits: DblpHits,
}

#[derive(Deserialize, Debug)]
struct DblpHits {
    hit: Vec<DblpHit>,
}

#[derive(Deserialize, Debug)]
struct DblpHit {
    info: DblpHitInfo,
}

#[derive(Deserialize, Debug, Clone)]
struct DblpHitInfo {
    key: String,
    authors: DblpAuthorEntry,
    title: String,
    venue: String,
    year: String,
    url: String,
}

impl DblpHitInfo {
    fn bib_url(&self, bibtype: Format) -> String {
        match bibtype {
            Format::Standard => format!("{}.bib?param=1", self.url),
            Format::Condensed => format!("{}.bib?param=0", self.url),
        }
    }

    fn get_key(&self) -> String {
        format!("DBLP:{}", self.key)
    }
}

fn bold(s: &str) -> String {
    format!("\x1b[1m{}\x1b[0m", s)
}

fn underline(s: &str) -> String {
    format!("\x1b[4m{}\x1b[0m", s)
}

impl SkimItem for DblpHitInfo {
    fn text(&self) -> Cow<'_, str> {
        Cow::Owned(format!(
            "{} {}",
            self.title,
            self.authors.as_vec().join(" ")
        ))
    }

    fn display<'a>(&'a self, _context: DisplayContext<'a>) -> AnsiString<'a> {
        AnsiString::from(self.title.clone())
    }

    fn preview(&self, _context: PreviewContext) -> ItemPreview {
        ItemPreview::AnsiText(format!(
            "{}\n{}\n{} {}",
            underline(&self.authors.as_vec().join(", ")),
            bold(&self.title),
            self.venue,
            self.year
        ))
    }
}

#[derive(Deserialize, Debug, Clone)]
struct DblpAuthor {
    #[serde(rename = "text")]
    name: String,
}

#[derive(Deserialize, Debug, Clone)]
struct DblpAuthorEntry {
    author: DblpAuthorList,
}

impl DblpAuthorEntry {
    fn as_vec(&self) -> Vec<String> {
        match &self.author {
            DblpAuthorList::Single(author) => vec![author.name.clone()],
            DblpAuthorList::List(authors) => authors.iter().map(|a| a.name.clone()).collect(),
        }
    }
}

#[derive(Deserialize, Debug, Clone)]
#[serde(untagged)]
enum DblpAuthorList {
    Single(DblpAuthor),
    List(Vec<DblpAuthor>),
}

/// gets the path to the only bibtex file in a directory. If there is none
/// or if there are multiple, return None
fn get_unique_bib() -> Result<Option<PathBuf>> {
    let paths: Vec<PathBuf> = std::fs::read_dir(".")?
        .filter_map(|s| {
            let p = s.as_ref().unwrap().path();
            if let Some(ext) = p.extension() {
                if ext == "bib" {
                    return Some(p);
                }
            }
            None
        })
        .collect();

    if paths.len() == 1 {
        Ok(Some(paths[0].clone()))
    } else {
        Ok(None)
    }
}

#[derive(Parser)]
struct Cli {
    #[command(subcommand)]
    subcommand: Actions,

    #[arg(short, long, value_name = "FILE")]
    bibtex: Option<String>,
}

impl Cli {
    fn get_bib_path(&self) -> Result<PathBuf> {
        self.bibtex
            .as_ref()
            .map(|s| PathBuf::from(s))
            .or_else(|| get_unique_bib().unwrap())
            .context("missing bibtex file")
    }

    fn get_backup_bib_path(&self) -> Result<PathBuf> {
        let orig = self.get_bib_path()?;
        Ok(orig.with_extension("bib.bak"))
    }
}

#[derive(Subcommand)]
enum Actions {
    Add { query: Vec<String> },
    Convert { to: Format },
}

#[derive(Copy, Clone, PartialEq, Eq, PartialOrd, Ord, ValueEnum)]
enum Format {
    Condensed,
    Standard,
}

impl Format {
    fn get_param(&self) -> &str {
        match self {
            Format::Standard => "?param=1",
            Format::Condensed => "?param=0",
        }
    }
}

fn write_clipboard(what: &str) -> Result<()> {
    fn run(cmd: &str, what: &str) -> Result<()> {
        let mut child = std::process::Command::new(cmd)
            .stdin(std::process::Stdio::piped())
            .spawn()?;
        write!(child.stdin.take().context("no standard input")?, "{}", what)?;
        Ok(())
    }
    ["wl-copy", "pbcopy"]
        .iter()
        .map(|cmd| run(cmd, what))
        .next()
        .context("no clipboard command ran successfully")?
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let bib_path = cli.get_bib_path()?;

    match cli.subcommand {
        Actions::Add { query } => {
            let query: Vec<&str> = query
                .iter()
                .flat_map(|v| v.split(" "))
                .map(|v| v.trim())
                .collect();
            let bibformat = Format::Condensed;
            let query = query.join("+");
            let resp: DblpResponse = URLS
                .iter()
                .map(|url| {
                    let url = format!(
                        "{}/search/publ/api?q={}&format=json&{}",
                        url,
                        query,
                        bibformat.get_param()
                    );
                    ureq::get(&url).call()
                })
                .filter(|r| r.is_ok())
                .next()
                .context("no successful response")??
                .into_json()?;

            let selection = show_and_select(resp.matches())?;

            if !is_present(&bib_path, &selection)? {
                let bib = ureq::get(&selection.bib_url(Format::Standard))
                    .call()?
                    .into_string()?;
                let mut writer = OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&bib_path)?;
                writeln!(writer, "{}", bib)?;
            }
            write_clipboard(&format!("DBLP:{}", selection.key))?;
        }
        Actions::Convert { to } => {
            let mut f = File::open(&bib_path)?;
            let mut src = String::new();
            f.read_to_string(&mut src)?;
            drop(f);

            // backup the content
            let mut f = File::create(cli.get_backup_bib_path()?)?;
            writeln!(f, "{}", src)?;
            drop(f);

            // overwrite the file
            let mut f = File::create(bib_path)?;

            let bibliography = Bibliography::parse(&src).unwrap();
            for entry in bibliography.iter() {
                if entry.key.starts_with("DBLP") {
                    let k = entry.key.replace("DBLP:", "");
                    let url = format!("https://dblp.uni-trier.de/rec/{}.bib{}", k, to.get_param());
                    let bib = ureq::get(&url).call()?.into_string()?;
                    writeln!(f, "{}\n", bib)?;
                } else {
                    writeln!(f, "{}\n", entry.to_bibtex_string().map_err(|e| anyhow!(e))?)?;
                }
            }
        }
    }

    Ok(())
}

fn is_present(path: &PathBuf, item: &DblpHitInfo) -> Result<bool> {
    let bib_key = item.get_key();

    if path.is_file() {
        let reader = BufReader::new(File::open(&path)?);
        for line in reader.lines() {
            let line = line?;
            if line.contains(&bib_key) {
                return Ok(true);
            }
        }
    }
    return Ok(false);
}

// copied from https://github.com/Mountlex/xivar/blob/main/src/finder.rs
fn show_and_select<I, T>(iter: T) -> Result<I>
where
    T: Iterator<Item = I>,
    I: SkimItem + Clone,
{
    let options = SkimOptionsBuilder::default()
        .height(Some("100%"))
        .preview(Some(""))
        .build()
        .expect("building fuzzy selector");

    let (tx_item, rx_item): (SkimItemSender, SkimItemReceiver) = unbounded();
    for item in iter {
        let _ = tx_item.send(Arc::new(item));
    }

    drop(tx_item); // so that skim could know when to stop waiting for more items.

    if let Some(output) = Skim::run_with(&options, Some(rx_item)) {
        if !output.is_abort {
            output
                .selected_items
                .into_iter()
                .nth(0)
                .map(move |item| {
                    (*item)
                        .as_any()
                        .downcast_ref::<I>() // downcast to concrete type
                        .expect("something wrong with downcast")
                        .clone()
                })
                .ok_or(anyhow!("Internal error"))
        } else {
            bail!("No entry selected! Aborting...")
        }
    } else {
        bail!("Internal error")
    }
}
