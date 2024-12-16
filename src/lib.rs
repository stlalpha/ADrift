use anyhow::{Context, Result};
use indicatif::{ProgressBar, ProgressStyle};
use std::path::Path;
use std::process::Command;
use std::time::Duration;
use std::sync::mpsc;
use std::thread;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::collections::hash_map::DefaultHasher;
mod db;
use db::FingerprintDb;

// Constants for segment detection
const COMMERCIAL_LENGTHS: &[f64] = &[15.0, 30.0, 60.0];
const STATION_ID_LENGTHS: &[f64] = &[3.0, 5.0, 10.0];
const COMMERCIAL_TOLERANCE: f64 = 1.0;
const STATION_ID_TOLERANCE: f64 = 0.2;  // Tighter tolerance for station IDs
const SIMILARITY_THRESHOLD: f64 = 0.95;

#[derive(Debug, Clone)]
pub struct SegmentFingerprint {
    duration: f64,
    audio_hash: u64,
    video_hash: u64,
}

impl PartialEq for SegmentFingerprint {
    fn eq(&self, other: &Self) -> bool {
        (self.audio_hash == other.audio_hash) && 
        (self.video_hash == other.video_hash) && 
        ((self.duration - other.duration).abs() < 0.001)  // Compare floats with tolerance
    }
}

impl Eq for SegmentFingerprint {}

impl Hash for SegmentFingerprint {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.audio_hash.hash(state);
        self.video_hash.hash(state);
        // Hash the duration as an integer milliseconds to avoid float issues
        ((self.duration * 1000.0) as i64).hash(state);
    }
}

#[derive(Debug)]
pub enum Segment {
    Commercial {
        start_time: f64,
        end_time: f64,
        duration: f64,
        fingerprint: Option<SegmentFingerprint>,
        duplicate_of: Option<i64>,
    },
    StationId {
        start_time: f64,
        end_time: f64,
        duration: f64,
        fingerprint: Option<SegmentFingerprint>,
        duplicate_of: Option<i64>,
    }
}

pub fn detect_commercials(
    input: &Path,
    black_threshold: f32,
    min_black_frames: u32,
    db_path: Option<&Path>,
) -> Result<Vec<Segment>> {
    println!("Analyzing video for commercial breaks and station IDs...");
    
    let duration = get_video_duration(input)?;
    let pb = ProgressBar::new(duration as u64);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("[{elapsed_precise}] {bar:40.cyan/blue} {pos:>7}/{len:7} {msg}")?
            .progress_chars("=>-")
    );

    let mut child = Command::new("ffmpeg")
        .args(&[
            "-i", input.to_str().unwrap(),
            "-vf", &format!("blackdetect=d=0.1:pic_th={}:pix_th={},select='not(mod(n,5))'", // Sample every 5th frame
                black_threshold, black_threshold),
            "-an",
            "-f", "null",
            "-progress", "pipe:1",
            "-"
        ])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .context("Failed to start FFmpeg")?;

    let stderr = child.stderr.take().unwrap();
    let stdout = child.stdout.take().unwrap();
    
    let stdout_reader = std::io::BufReader::new(stdout);
    let stderr_reader = std::io::BufReader::new(stderr);
    
    use std::io::BufRead;
    
    // Create channels for communication between threads
    let (tx_segments, rx_segments) = mpsc::channel();
    let (tx_progress, rx_progress) = mpsc::channel();

    // Spawn thread for stderr processing (black frame detection)
    let stderr_thread = thread::spawn(move || {
        for line in stderr_reader.lines() {
            if let Ok(line) = line {
                if line.contains("blackdetect") {
                    if let Some((start, end)) = parse_black_frame_line(&line) {
                        let duration = end - start;
                        if STATION_ID_LENGTHS.iter().any(|&len| (duration - len).abs() < STATION_ID_TOLERANCE) {
                            tx_progress.send(format!("Potential station ID at {:.1}s", start)).ok();
                        }
                        tx_segments.send((start, end)).ok();
                    }
                }
            }
        }
    });

    // Process progress updates in main thread
    let mut potential_segments = Vec::new();
    let mut stderr_done = false;
    let mut lines = stdout_reader.lines();
    
    while !stderr_done {
        // Check for progress updates
        if let Some(Ok(line)) = lines.next() {
            if line.starts_with("out_time_ms=") {
                if let Ok(time) = line[12..].parse::<u64>() {
                    pb.set_position(time / 1_000_000);
                    pb.set_message("Analyzing...");
                }
            }
        }

        // Check for segment updates
        match rx_segments.try_recv() {
            Ok((start, end)) => potential_segments.push((start, end)),
            Err(mpsc::TryRecvError::Disconnected) => stderr_done = true,
            Err(mpsc::TryRecvError::Empty) => {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        }

        // Check for status messages
        if let Ok(msg) = rx_progress.try_recv() {
            pb.set_message(msg);
        }
    }

    // Wait for stderr thread to finish
    stderr_thread.join().unwrap();

    pb.finish_with_message("Analysis complete");
    
    // Add debug information
    println!("Found {} potential black frame segments", potential_segments.len());
    
    if potential_segments.is_empty() {
        println!("Warning: No black frames detected with threshold {}. Try adjusting the black_threshold parameter.", black_threshold);
        // Return empty results instead of error
        return Ok(Vec::new());
    }
    
    let db = db_path.map(|path| FingerprintDb::new(path))
        .transpose()?;

    let segments = identify_commercials(
        potential_segments,
        input,
        min_black_frames,
        db.as_ref(),
    )?;
    
    // Add debug output to see what segments were found
    println!("\nDetected segments:");
    for segment in &segments {
        match segment {
            Segment::Commercial { start_time, end_time, duration, .. } => {
                println!("Commercial: start={:.1}s, end={:.1}s, duration={:.1}s", 
                    start_time, end_time, duration);
            },
            Segment::StationId { start_time, end_time, duration, .. } => {
                println!("Station ID: start={:.1}s, end={:.1}s, duration={:.1}s", 
                    start_time, end_time, duration);
            }
        }
    }
    
    // Print summary with unique count
    let (commercials, station_ids): (Vec<_>, Vec<_>) = segments.iter().partition(|s| matches!(s, Segment::Commercial { .. }));
    println!("\nFound {} unique commercials and {} unique station IDs", 
             commercials.len(), 
             station_ids.len());
    
    Ok(segments)
}

// Helper function to parse a single black frame line
fn parse_black_frame_line(line: &str) -> Option<(f64, f64)> {
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
            return Some((start, end));
        }
    }
    None
}

pub fn extract_segment(
    input: &Path,
    output_dir: &Path,
    index: usize,
    commercial_total: usize,
    station_id_total: usize,
    segment: &Segment,
    output_ext: &str,
) -> Result<()> {
    let input_stem = input
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown");

    let (start_time, end_time, prefix, current, total) = match segment {
        Segment::Commercial { start_time, end_time, duration: _, fingerprint: _, .. } => {
            (*start_time, *end_time, "commercial", index + 1, commercial_total)
        },
        Segment::StationId { start_time, end_time, duration: _, fingerprint: _, .. } => {
            (*start_time, *end_time, "station-id", index + 1, station_id_total)
        }
    };

    let output_path = output_dir.join(format!("{}-{}-{}.{}", 
        input_stem,
        prefix,
        current,
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
    pb.set_message(format!("Extracting {} {}/{}", prefix, current, total));
    pb.enable_steady_tick(Duration::from_millis(100));

    Command::new("ffmpeg")
        .args(&command_args)
        .output()
        .with_context(|| format!("Failed to extract {} {}/{}", prefix, current, total))?;
    
    pb.finish_with_message(format!("Extracted {} {}/{}", prefix, current, total));
    
    Ok(())
}

fn generate_segment_fingerprint(input: &Path, start_time: f64, end_time: f64) -> Result<SegmentFingerprint> {
    // Generate a low-res video hash with more efficient settings
    let video_hash = Command::new("ffmpeg")
        .args(&[
            "-i", input.to_str().unwrap(),
            "-ss", &start_time.to_string(),
            "-t", &(end_time - start_time).to_string(),
            "-vf", "scale=16:16,format=gray", // Reduce resolution further
            "-frames:v", "1", // Only take one frame
            "-f", "rawvideo",
            "-loglevel", "error",
            "-"
        ])
        .stderr(std::process::Stdio::piped())
        .output()
        .context("Failed to generate video hash")?;

    if !video_hash.status.success() {
        let error = String::from_utf8_lossy(&video_hash.stderr);
        return Err(anyhow::anyhow!("FFmpeg video hash failed: {}", error));
    }

    // Generate audio fingerprint with more efficient settings
    let audio_hash = Command::new("ffmpeg")
        .args(&[
            "-i", input.to_str().unwrap(),
            "-ss", &start_time.to_string(),
            "-t", &(end_time - start_time).to_string(),
            "-ac", "1",
            "-ar", "4000", // Lower sample rate
            "-f", "s16le",
            "-loglevel", "error",
            "-"
        ])
        .stderr(std::process::Stdio::piped())
        .output()
        .context("Failed to generate audio hash")?;

    if !audio_hash.status.success() {
        let error = String::from_utf8_lossy(&audio_hash.stderr);
        return Err(anyhow::anyhow!("FFmpeg audio hash failed: {}", error));
    }

    // Create hashes with error handling
    let video_hash = if !video_hash.stdout.is_empty() {
        let mut hasher = DefaultHasher::new();
        video_hash.stdout.hash(&mut hasher);
        hasher.finish()
    } else {
        let mut hasher = DefaultHasher::new();
        "no_video_data".hash(&mut hasher);
        hasher.finish()
    };

    let audio_hash = if !audio_hash.stdout.is_empty() {
        let mut hasher = DefaultHasher::new();
        audio_hash.stdout.hash(&mut hasher);
        hasher.finish()
    } else {
        let mut hasher = DefaultHasher::new();
        "no_audio_data".hash(&mut hasher);
        hasher.finish()
    };

    Ok(SegmentFingerprint {
        duration: end_time - start_time,
        audio_hash,
        video_hash,
    })
}

fn identify_commercials(
    black_frames: Vec<(f64, f64)>,
    input: &Path,
    _min_black_frames: u32,
    db: Option<&FingerprintDb>,
) -> Result<Vec<Segment>> {
    if black_frames.is_empty() {
        return Ok(Vec::new());
    }

    let mut segments = Vec::new();
    let mut unique_fingerprints: HashMap<SegmentFingerprint, usize> = HashMap::new();
    
    let pb = ProgressBar::new((black_frames.len().saturating_sub(1)) as u64);
    pb.set_style(
        ProgressStyle::default_bar()
            .template("[{elapsed_precise}] {bar:40.cyan/blue} {pos:>7}/{len:7} {msg}")?
            .progress_chars("=>-")
    );
    pb.set_message("Generating fingerprints...");

    // Only process if we have at least 2 black frames
    if black_frames.len() >= 2 {
        for i in 0..black_frames.len() - 1 {
            let current_end = black_frames[i].1;
            let next_start = black_frames[i + 1].0;
            let duration = next_start - current_end;
            
            println!("Checking segment: duration={:.1}s", duration);  // Debug output
            
            match generate_segment_fingerprint(input, current_end, next_start) {
                Ok(fingerprint) => {
                    // Check if it matches commercial lengths
                    let is_commercial = COMMERCIAL_LENGTHS.iter()
                        .any(|&len| (duration - len).abs() < COMMERCIAL_TOLERANCE);
                    let is_station_id = STATION_ID_LENGTHS.iter()
                        .any(|&len| (duration - len).abs() < STATION_ID_TOLERANCE);
                    
                    println!("  Is commercial? {} (duration={:.1}s)", is_commercial, duration);
                    println!("  Is station ID? {} (duration={:.1}s)", is_station_id, duration);

                    // Check database first if available
                    if let Some(db) = db {
                        if let Some(existing_id) = db.find_similar_fingerprint(&fingerprint, SIMILARITY_THRESHOLD)? {
                            db.update_fingerprint_occurrence(existing_id)?;
                            
                            // Get the type of the existing fingerprint
                            if let Some(segment_type) = db.get_fingerprint_type(existing_id)? {
                                // Add to segments based on the stored type
                                match segment_type.as_str() {
                                    "commercial" => {
                                        segments.push(Segment::Commercial {
                                            start_time: current_end,
                                            end_time: next_start,
                                            duration,
                                            fingerprint: Some(fingerprint),
                                            duplicate_of: Some(existing_id),
                                        });
                                    },
                                    "station_id" => {
                                        segments.push(Segment::StationId {
                                            start_time: current_end,
                                            end_time: next_start,
                                            duration,
                                            fingerprint: Some(fingerprint),
                                            duplicate_of: Some(existing_id),
                                        });
                                    },
                                    _ => {}
                                }
                            }
                            pb.inc(1);
                            continue;
                        }
                    }

                    // Process new segments
                    if is_station_id {
                        if !unique_fingerprints.contains_key(&fingerprint) {
                            if let Some(db) = db {
                                db.store_fingerprint(&fingerprint, "station_id")?;
                            }
                            unique_fingerprints.insert(fingerprint.clone(), segments.len());
                            segments.push(Segment::StationId {
                                start_time: current_end,
                                end_time: next_start,
                                duration,
                                fingerprint: Some(fingerprint),
                                duplicate_of: None,
                            });
                        }
                    } else if is_commercial {
                        if !unique_fingerprints.contains_key(&fingerprint) {
                            if let Some(db) = db {
                                db.store_fingerprint(&fingerprint, "commercial")?;
                            }
                            unique_fingerprints.insert(fingerprint.clone(), segments.len());
                            segments.push(Segment::Commercial {
                                start_time: current_end,
                                end_time: next_start,
                                duration,
                                fingerprint: Some(fingerprint),
                                duplicate_of: None,
                            });
                        }
                    }
                },
                Err(e) => {
                    eprintln!("Error generating fingerprint at {:.1}s: {}", current_end, e);
                }
            }
            pb.inc(1);
        }
    }
    
    pb.finish_with_message("Fingerprint generation complete");
    Ok(segments)
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