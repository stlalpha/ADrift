use adrift::{Segment, detect_commercials, extract_segment};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use structopt::StructOpt;

const BUILD_VERSION: &str = env!("BUILD_VERSION");

#[derive(Debug, Clone)]
enum OutputFormat {
    Same,
    MP4,
    WebM,
    MKV,
    MOV,
}

impl FromStr for OutputFormat {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "same" => Ok(OutputFormat::Same),
            "mp4" => Ok(OutputFormat::MP4),
            "webm" => Ok(OutputFormat::WebM),
            "mkv" => Ok(OutputFormat::MKV),
            "mov" => Ok(OutputFormat::MOV),
            _ => Err(format!("Unsupported format: {}. Supported formats are: same, mp4, webm, mkv, mov", s)),
        }
    }
}

#[derive(Debug, StructOpt)]
#[structopt(
    name = "adrift",
    about = "ADrift - A tool for discovering and preserving commercials and station IDs from the past"
)]
struct Opt {
    #[structopt(parse(from_os_str), help = "Input video file or directory")]
    input: PathBuf,
    
    #[structopt(parse(from_os_str), help = "Output directory for extracted segments")]
    output_dir: PathBuf,
    
    #[structopt(long, default_value = "0.1", help = "Threshold for black frame detection (0.0-1.0)")]
    black_threshold: f32,
    
    #[structopt(long, default_value = "3", help = "Minimum number of black frames for detection")]
    min_black_frames: u32,

    #[structopt(long, default_value = "same", help = "Output format: same, mp4, webm, mkv, mov")]
    output_format: OutputFormat,

    #[structopt(long, help = "Process recursively if input is a directory")]
    recursive: bool,

    #[structopt(long, help = "File extensions to process (comma-separated, e.g. 'mp4,avi,mkv')")]
    extensions: Option<String>,

    #[structopt(long, help = "Enable verbose output")]
    verbose: bool,

    #[structopt(long, parse(from_os_str), help = "Path to SQLite database for fingerprint storage")]
    db_path: Option<PathBuf>,
}

fn get_output_extension(input_path: &Path, output_format: &OutputFormat) -> String {
    match output_format {
        OutputFormat::Same => input_path
            .extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| ext.to_string())
            .unwrap_or_else(|| "mp4".to_string()),
        OutputFormat::MP4 => "mp4".to_string(),
        OutputFormat::WebM => "webm".to_string(),
        OutputFormat::MKV => "mkv".to_string(),
        OutputFormat::MOV => "mov".to_string(),
    }
}

fn process_file(
    input: &Path,
    output_dir: &Path,
    black_threshold: f32,
    min_black_frames: u32,
    output_format: &OutputFormat,
    db_path: Option<&Path>,
) -> Result<(), Box<dyn std::error::Error>> {
    println!("\nProcessing: {}", input.display());
    
    let output_ext = get_output_extension(input, output_format);
    let segments = detect_commercials(
        input, 
        black_threshold, 
        min_black_frames,
        db_path,
    )?;
    
    let (commercials, station_ids): (Vec<_>, Vec<_>) = segments.iter().partition(|s| matches!(s, Segment::Commercial { .. }));
    let commercial_count = commercials.len();
    let station_id_count = station_ids.len();

    let mut commercial_index = 0;
    let mut station_id_index = 0;

    for segment in segments.iter() {
        match segment {
            Segment::Commercial { .. } => {
                extract_segment(
                    input,
                    output_dir,
                    commercial_index,
                    commercial_count,
                    station_id_count,
                    segment,
                    &output_ext,
                )?;
                commercial_index += 1;
            },
            Segment::StationId { .. } => {
                extract_segment(
                    input,
                    output_dir,
                    station_id_index,
                    commercial_count,
                    station_id_count,
                    segment,
                    &output_ext,
                )?;
                station_id_index += 1;
            }
        }
    }
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let opt = Opt::from_args();
    
    println!("ADrift v{} (build {})", env!("CARGO_PKG_VERSION"), BUILD_VERSION);
    println!("----------------------------------------");
    
    std::fs::create_dir_all(&opt.output_dir)?;
    
    let extensions: Vec<String> = opt.extensions
        .as_deref()
        .unwrap_or("mp4,avi,mkv,mov,wmv,webm")
        .split(',')
        .map(|s| s.trim().to_lowercase())
        .collect();

    if opt.verbose {
        println!("Looking for files with extensions: {:?}", extensions);
    }

    let should_process = |path: &Path| -> bool {
        path.extension()
            .and_then(|ext| ext.to_str())
            .map(|ext| extensions.contains(&ext.to_lowercase()))
            .unwrap_or(false)
    };

    if opt.input.is_file() {
        if should_process(&opt.input) {
            process_file(
                &opt.input,
                &opt.output_dir,
                opt.black_threshold,
                opt.min_black_frames,
                &opt.output_format,
                opt.db_path.as_deref(),
            )?;
        } else {
            println!("Skipping unsupported file: {}", opt.input.display());
        }
    } else if opt.input.is_dir() {
        let mut files: Vec<PathBuf> = Vec::new();
        
        let walker = if opt.recursive {
            walkdir::WalkDir::new(&opt.input)
        } else {
            walkdir::WalkDir::new(&opt.input).max_depth(1)
        };

        for entry in walker.into_iter().filter_map(|e| e.ok()) {
            let path = entry.path().to_path_buf();
            if path.is_file() && should_process(&path) {
                if opt.verbose {
                    println!("Found video file: {}", path.display());
                }
                files.push(path);
            }
        }

        if files.is_empty() {
            println!("No video files found! Make sure your files have one of these extensions: {:?}", extensions);
            return Ok(());
        }

        println!("Found {} files to process", files.len());
        
        files.sort();

        let progress = indicatif::ProgressBar::new(files.len() as u64);
        progress.set_style(
            indicatif::ProgressStyle::default_bar()
                .template("[{elapsed_precise}] {bar:40.cyan/blue} {pos}/{len} {msg}")?
                .progress_chars("=>-")
        );

        for file in files {
            progress.set_message(format!("Processing: {}", file.display()));
            if let Err(e) = process_file(
                &file,
                &opt.output_dir,
                opt.black_threshold,
                opt.min_black_frames,
                &opt.output_format,
                opt.db_path.as_deref(),
            ) {
                eprintln!("Error processing {}: {}", file.display(), e);
            }
            progress.inc(1);
        }

        progress.finish_with_message("Batch processing complete");
    }
    
    Ok(())
} 