mod lib;
use lib::{Commercial, detect_commercials, extract_commercial};
use std::path::PathBuf;
use std::str::FromStr;
use structopt::StructOpt;

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
    about = "ADrift - A tool for discovering and preserving commercials from the past"
)]
struct Opt {
    #[structopt(parse(from_os_str), help = "Input video file")]
    input: PathBuf,
    
    #[structopt(parse(from_os_str), help = "Output directory for extracted commercials")]
    output_dir: PathBuf,
    
    #[structopt(long, default_value = "0.1", help = "Threshold for black frame detection (0.0-1.0)")]
    black_threshold: f32,
    
    #[structopt(long, default_value = "3", help = "Minimum number of black frames for detection")]
    min_black_frames: u32,

    #[structopt(long, default_value = "same", help = "Output format: same, mp4, webm, mkv, mov")]
    output_format: OutputFormat,
}

fn get_output_extension(input_path: &PathBuf, output_format: &OutputFormat) -> String {
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

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let opt = Opt::from_args();
    
    // Ensure output directory exists
    std::fs::create_dir_all(&opt.output_dir)?;
    
    // Get the output format extension
    let output_ext = get_output_extension(&opt.input, &opt.output_format);
    
    // Process the video
    let commercials = detect_commercials(&opt.input, opt.black_threshold, opt.min_black_frames)?;
    
    // Extract each commercial
    for (i, commercial) in commercials.iter().enumerate() {
        extract_commercial(
            &opt.input,
            &opt.output_dir,
            i,
            commercial.start_time,
            commercial.end_time,
            &output_ext,
        )?;
    }
    
    Ok(())
} 