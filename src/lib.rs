use anyhow::{Context, Result};
use indicatif::{ProgressBar, ProgressStyle};
use std::path::Path;
use std::process::Command;
use std::time::Duration;

#[derive(Debug)]
pub struct Commercial {
    pub start_time: f64,
    pub end_time: f64,
}

pub fn detect_commercials(
    input: &Path,
    black_threshold: f32,
    min_black_frames: u32,
) -> Result<Vec<Commercial>> {
    println!("Analyzing video for commercial breaks...");
    
    // Get video duration for progress bar
    let duration = get_video_duration(input)?;
    let pb = ProgressBar::new(duration as u64);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("[{elapsed_precise}] {bar:40.cyan/blue} {pos:>7}/{len:7} {msg}")?
            .progress_chars("=>-")
    );

    // Use ffmpeg with progress output
    let mut child = Command::new("ffmpeg")
        .args(&[
            "-i",
            input.to_str().unwrap(),
            "-vf",
            &format!("blackdetect=d=0.1:pix_th={}", black_threshold),
            "-f",
            "null",
            "-progress", "pipe:1",
            "-"
        ])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .context("Failed to start FFmpeg")?;

    let stderr = child.stderr.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    
    // Read progress from stdout
    let stdout_reader = std::io::BufReader::new(stdout);
    let stderr_reader = std::io::BufReader::new(stderr);
    
    use std::io::BufRead;
    
    // Update progress bar based on FFmpeg output
    for line in stdout_reader.lines() {
        if let Ok(line) = line {
            if line.starts_with("out_time_ms=") {
                if let Ok(time) = line[12..].parse::<u64>() {
                    pb.set_position(time / 1000000);
                }
            }
        }
    }

    // Collect stderr for black frame detection
    let stderr_output: Vec<String> = stderr_reader.lines()
        .filter_map(Result::ok)
        .collect();

    let black_frames = parse_black_frames(&stderr_output.join("\n"))?;
    pb.finish_with_message("Analysis complete");
    
    // Group black frames into potential commercial boundaries
    let commercials = identify_commercials(black_frames, min_black_frames);
    
    println!("Found {} potential commercials", commercials.len());
    
    Ok(commercials)
}

pub fn extract_commercial(
    input: &Path,
    output_dir: &Path,
    index: usize,
    start_time: f64,
    end_time: f64,
    output_ext: &str,
) -> Result<()> {
    let input_stem = input
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown");

    let output_path = output_dir.join(format!("{}-commercial-{}.{}", 
        input_stem,
        index,
        output_ext
    ));
    
    let codec_args = match output_ext {
        "mov" => vec!["-c:v", "h264", "-c:a", "aac"],
        "mp4" => vec!["-c:v", "h264", "-c:a", "aac"],
        _ => vec!["-c", "copy"],
    };
    
    let start_time_str = start_time.to_string();
    let duration_str = (end_time - start_time).to_string();
    
    let mut command_args = vec![
        "-i", input.to_str().unwrap(),
        "-ss", &start_time_str,
        "-t", &duration_str,
    ];
    command_args.extend(codec_args);
    command_args.push(output_path.to_str().unwrap());

    let pb = ProgressBar::new_spinner();
    pb.set_style(
        ProgressStyle::default_spinner()
            .template("{spinner:.green} {msg}")?
            .tick_chars("⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏")
    );
    pb.set_message(format!("Extracting commercial {}", index + 1));
    pb.enable_steady_tick(Duration::from_millis(100));

    Command::new("ffmpeg")
        .args(&command_args)
        .output()
        .with_context(|| format!("Failed to extract commercial {}", index + 1))?;
    
    pb.finish_with_message(format!("Extracted commercial {}", index + 1));
    
    Ok(())
}

fn parse_black_frames(ffmpeg_output: &str) -> Result<Vec<(f64, f64)>> {
    let mut sequences = Vec::new();
    
    for line in ffmpeg_output.lines() {
        if line.contains("blackdetect") {
            // Parse black frame timestamps
            // Example: [blackdetect @ 0x7f8f9c006800] black_start:10 black_end:12
            if let (Some(start), Some(end)) = (
                line.find("black_start:").map(|i| {
                    line[i..].split(':').nth(1)
                        .and_then(|s| s.split_whitespace().next())
                        .and_then(|s| s.parse::<f64>().ok())
                }),
                line.find("black_end:").map(|i| {
                    line[i..].split(':').nth(1)
                        .and_then(|s| s.split_whitespace().next())
                        .and_then(|s| s.parse::<f64>().ok())
                })
            ) {
                if let (Some(start), Some(end)) = (start, end) {
                    sequences.push((start, end));
                }
            }
        }
    }
    
    Ok(sequences)
}

fn identify_commercials(
    black_frames: Vec<(f64, f64)>,
    _min_black_frames: u32,
) -> Vec<Commercial> {
    let mut commercials = Vec::new();
    
    // Standard commercial lengths in seconds
    const COMMERCIAL_LENGTHS: &[f64] = &[15.0, 30.0, 60.0];
    const TOLERANCE: f64 = 1.0; // 1 second tolerance

    // Look at each pair of black frame sequences
    for i in 0..black_frames.len() - 1 {
        let current_end = black_frames[i].1;     // End of current black frame
        let next_start = black_frames[i + 1].0;  // Start of next black frame
        
        let segment_duration = next_start - current_end;
        
        // If this segment matches a standard commercial length
        if COMMERCIAL_LENGTHS.iter().any(|&len| (segment_duration - len).abs() < TOLERANCE) {
            commercials.push(Commercial {
                start_time: current_end,
                end_time: next_start,
            });
        }
    }
    
    commercials
}

fn get_video_duration(path: &Path) -> Result<f64> {
    let output = Command::new("ffprobe")
        .args(&[
            "-v", "error",
            "-show_entries", "format=duration",
            "-of", "default=noprint_wrappers=1:nokey=1",
            path.to_str().unwrap()
        ])
        .output()
        .context("Failed to get video duration")?;
    
    String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse::<f64>()
        .context("Failed to parse video duration")
} 