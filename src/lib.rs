use std::path::Path;
use std::process::Command;

#[derive(Debug)]
pub struct BlackFrameSequence {
    pub start_time: f64,
    pub end_time: f64,
}

#[derive(Debug)]
pub struct Commercial {
    pub start_time: f64,
    pub end_time: f64,
}

pub fn detect_commercials(
    input: &Path,
    black_threshold: f32,
    min_black_frames: u32,
) -> Result<Vec<Commercial>, Box<dyn std::error::Error>> {
    // Use ffmpeg to detect black frames
    let output = Command::new("ffmpeg")
        .args(&[
            "-i",
            input.to_str().unwrap(),
            "-vf",
            &format!("blackdetect=d=0.1:pix_th={}", black_threshold),
            "-f",
            "null",
            "-"
        ])
        .output()?;

    let black_frames = parse_black_frames(&String::from_utf8_lossy(&output.stderr))?;
    
    // Group black frames into potential commercial boundaries
    let commercials = identify_commercials(black_frames, min_black_frames);
    
    Ok(commercials)
}

pub fn extract_commercial(
    input: &Path,
    output_dir: &Path,
    index: usize,
    start_time: f64,
    end_time: f64,
    output_ext: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let output_path = output_dir.join(format!("commercial_{}.{}", index, output_ext));
    
    let codec_args = match output_ext {
        "mov" => vec!["-c:v", "h264", "-c:a", "aac"],
        "mp4" => vec!["-c:v", "h264", "-c:a", "aac"],
        _ => vec!["-c", "copy"],  // Default to stream copy for other formats
    };
    
    // Create strings before building the command args
    let start_time_str = start_time.to_string();
    let duration_str = (end_time - start_time).to_string();
    
    let mut command_args = vec![
        "-i", input.to_str().unwrap(),
        "-ss", &start_time_str,
        "-t", &duration_str,
    ];
    command_args.extend(codec_args);
    command_args.push(output_path.to_str().unwrap());

    Command::new("ffmpeg")
        .args(&command_args)
        .output()?;
    
    Ok(())
}

fn parse_black_frames(ffmpeg_output: &str) -> Result<Vec<BlackFrameSequence>, Box<dyn std::error::Error>> {
    // Parse ffmpeg blackdetect output
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
                    sequences.push(BlackFrameSequence {
                        start_time: start,
                        end_time: end,
                    });
                }
            }
        }
    }
    
    Ok(sequences)
}

fn identify_commercials(
    black_frames: Vec<BlackFrameSequence>,
    _min_black_frames: u32,
) -> Vec<Commercial> {
    let mut commercials = Vec::new();
    let mut current_start = None;
    
    // Standard commercial lengths in seconds
    const COMMERCIAL_LENGTHS: &[f64] = &[15.0, 30.0, 60.0];
    const TOLERANCE: f64 = 1.0; // 1 second tolerance
    
    for window in black_frames.windows(2) {
        let duration = window[1].start_time - window[0].end_time;
        
        // Check if duration matches standard commercial length
        if COMMERCIAL_LENGTHS.iter().any(|&len| (duration - len).abs() < TOLERANCE) {
            if current_start.is_none() {
                current_start = Some(window[0].end_time);
            }
        } else if let Some(start) = current_start.take() {
            commercials.push(Commercial {
                start_time: start,
                end_time: window[0].start_time,
            });
        }
    }
    
    commercials
} 