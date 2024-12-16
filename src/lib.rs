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
const COMMERCIAL_LENGTHS: &[f64] = &[15.0, 20.0, 25.0, 30.0, 45.0, 60.0];
const STATION_ID_LENGTHS: &[f64] = &[3.0, 5.0, 10.0];
const COMMERCIAL_TOLERANCE: f64 = 2.0;
const STATION_ID_TOLERANCE: f64 = 0.2;  // Tighter tolerance for station IDs
const SIMILARITY_THRESHOLD: f64 = 0.95;

// Constants for black frame detection
const MIN_BLACK_DURATION: f64 = 0.016;  // About half a frame at 29.97fps
const BLACK_PIXEL_THRESHOLD: f64 = 0.15; // 15% brightness threshold
const MIN_SIGNIFICANT_BLACK: f64 = 0.1;  // Minimum duration for "significant" black frames
const MAX_BLACK_FRAME_GAP: f64 = 0.5;  // Maximum gap between related black frames (500ms)

#[derive(Debug, Clone)]
pub struct BlackFrameConfig {
    min_duration: f64,      // Minimum duration of a black frame (default: 0.02s)
    max_brightness: f64,    // Maximum brightness to consider "black" (0-1)
    noise_tolerance: f64,   // Tolerance for noise in black frames (0-1)
    frame_skip: i32,        // Number of frames to skip in analysis
    min_gap: f64,          // Minimum gap between segments
    max_gap: f64,          // Maximum gap to consider part of same segment
}

impl Default for BlackFrameConfig {
    fn default() -> Self {
        Self {
            min_duration: 0.02,        // Comskip: minimum black frame duration
            max_brightness: 0.08,      // Comskip: stricter black level
            noise_tolerance: 0.08,     // Comskip: noise threshold
            frame_skip: 1,            // Check every frame
            min_gap: 0.1,            // Minimum gap between segments
            max_gap: 0.5,            // Maximum gap to merge segments
        }
    }
}

#[derive(Debug, Clone)]
pub struct SceneChangeConfig {
    threshold: f64,           // Pixel difference threshold (0.0-1.0)
    min_scene_length: f64,    // Minimum scene duration in seconds
}

impl Default for SceneChangeConfig {
    fn default() -> Self {
        Self {
            threshold: 0.4,        // Comskip's default scene change threshold
            min_scene_length: 0.5, // Minimum scene duration
        }
    }
}

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

#[derive(Debug)]
struct SegmentBoundary {
    time: f64,
    black_frame_score: f64,
    scene_change_score: f64,
    total_score: f64,
}

impl SegmentBoundary {
    fn new(time: f64) -> Self {
        Self {
            time,
            black_frame_score: 0.0,
            scene_change_score: 0.0,
            total_score: 0.0,
        }
    }

    fn update_scores(&mut self) {
        self.total_score = self.black_frame_score * 0.7 + 
                          self.scene_change_score * 0.3;
    }
}

fn check_ffmpeg_version() -> Result<()> {
    let output = Command::new("ffmpeg")
        .arg("-version")
        .output()
        .context("Failed to execute ffmpeg")?;

    let version_str = String::from_utf8_lossy(&output.stdout);
    if !version_str.contains("ffmpeg version 4.") {
        return Err(anyhow::anyhow!(
            "This version requires FFmpeg 4.x. Found:\n{}",
            version_str.lines().next().unwrap_or("unknown version")
        ));
    }
    Ok(())
}

pub fn detect_commercials(
    input: &Path,
    black_threshold: f32,
    min_black_frames: u32,
    db_path: Option<&Path>,
) -> Result<Vec<Segment>> {
    check_ffmpeg_version()?;
    
    // Initialize configs
    let black_config = BlackFrameConfig::default();
    let scene_config = SceneChangeConfig::default();

    println!("Analyzing video for commercial breaks...");
    let duration = get_video_duration(input)?;
    let pb = ProgressBar::new(duration as u64);
    pb.set_style(ProgressStyle::default_bar()
        .template("[{elapsed_precise}] {bar:40.cyan/blue} {pos:>7}/{len:7} {msg}")?);

    // Detect both black frames and scene changes
    println!("Detecting black frames...");
    let black_frames = detect_black_frames(input, &black_config)?;
    
    println!("Detecting scene changes...");
    let scene_changes = detect_scene_changes(input, &scene_config)?;

    // Debug output
    println!("\nDetection summary:");
    println!("  Found {} black frames", black_frames.len());
    println!("  Found {} scene changes", scene_changes.len());

    // Score and filter boundaries
    let scored_boundaries = score_segment_boundaries(
        &black_frames, 
        &scene_changes, 
        black_config.max_gap
    );
    
    println!("\nBoundary scoring summary:");
    for boundary in &scored_boundaries {
        println!("  Time: {} (Score: {:.2})", 
            format_timestamp(boundary.time),
            boundary.total_score);
        println!("    Black frame score: {:.2}", boundary.black_frame_score);
        println!("    Scene change score: {:.2}", boundary.scene_change_score);
    }

    // Convert scored boundaries to potential segments
    let mut potential_segments = Vec::new();
    if scored_boundaries.len() >= 2 {
        for pair in scored_boundaries.windows(2) {
            potential_segments.push((pair[0].time, pair[1].time));
        }
    }

    let db = db_path.map(|path| FingerprintDb::new(path))
        .transpose()?;

    let segments = identify_commercials(
        potential_segments,
        input,
        min_black_frames,
        db.as_ref(),
    )?;

    // Print summary
    println!("\nDetected segments:");
    for segment in &segments {
        match segment {
            Segment::Commercial { start_time, end_time, duration, .. } => {
                println!("Commercial: {} to {} (duration: {:.1}s)", 
                    format_timestamp(*start_time),
                    format_timestamp(*end_time),
                    duration);
            },
            Segment::StationId { start_time, end_time, duration, .. } => {
                println!("Station ID: {} to {} (duration: {:.1}s)", 
                    format_timestamp(*start_time),
                    format_timestamp(*end_time),
                    duration);
            }
        }
    }

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

    let mut cmd = Command::new("ffmpeg");
    cmd.args(&command_args);
    
    print_command(&cmd);
    
    cmd.output()
        .with_context(|| format!("Failed to extract {} {}/{}", prefix, current, total))?;
    
    pb.finish_with_message(format!("Extracted {} {}/{}", prefix, current, total));
    
    Ok(())
}

fn generate_segment_fingerprint(input: &Path, start_time: f64, end_time: f64) -> Result<SegmentFingerprint> {
    // Video hash command
    let mut cmd = Command::new("ffmpeg");
    cmd.args(&[
        "-i", input.to_str().unwrap(),
        "-ss", &start_time.to_string(),
        "-t", &(end_time - start_time).to_string(),
        "-vf", "scale=16:16,format=gray",
        "-frames:v", "1",
        "-f", "rawvideo",
        "-loglevel", "error",
        "-"
    ]);
    
    print_command(&cmd);
    let video_hash = cmd
        .stderr(std::process::Stdio::piped())
        .output()
        .context("Failed to generate video hash")?;

    // Audio hash command
    let mut cmd = Command::new("ffmpeg");
    cmd.args(&[
        "-i", input.to_str().unwrap(),
        "-ss", &start_time.to_string(),
        "-t", &(end_time - start_time).to_string(),
        "-ac", "1",
        "-ar", "4000",
        "-f", "s16le",
        "-loglevel", "error",
        "-"
    ]);
    
    print_command(&cmd);
    let audio_hash = cmd
        .stderr(std::process::Stdio::piped())
        .output()
        .context("Failed to generate audio hash")?;

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
            
            // Add detailed debugging for segments around our target area
            if (current_end >= 635.0 && current_end <= 645.0) || 
               (next_start >= 635.0 && next_start <= 645.0) {
                println!("\nDebug: [Target Region] Analyzing potential commercial:");
                println!("  Segment: {:.3}s to {:.3}s (duration: {:.3}s)", 
                    current_end, next_start, duration);
                println!("  Commercial match distances:");
                for &len in COMMERCIAL_LENGTHS {
                    println!("    {:.1}s commercial: diff = {:.3}s (tolerance: {:.1}s)", 
                        len, (duration - len).abs(), COMMERCIAL_TOLERANCE);
                }
            }
            
            println!("\nDebug: Analyzing segment boundaries:");
            println!("  Previous black frame end: {:.3}s", current_end);
            println!("  Next black frame start: {:.3}s", next_start);
            println!("  Segment duration: {:.3}s", duration);
            
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
    let mut cmd = Command::new("ffprobe");
    cmd.args(&[
        "-v", "error",
        "-show_entries", "format=duration",
        "-of", "default=noprint_wrappers=1:nokey=1",
        path.to_str().unwrap()
    ]);
    
    println!("Debug: Executing: ffprobe {}", 
        cmd.get_args()
            .map(|arg| arg.to_str().unwrap_or(""))
            .collect::<Vec<&str>>()
            .join(" ")
    );
    
    let output = cmd
        .output()
        .context("Failed to get video duration")?;
    
    String::from_utf8_lossy(&output.stdout)
        .trim()
        .parse::<f64>()
        .context("Failed to parse video duration")
}

fn print_command(cmd: &Command) {
    let args: Vec<&str> = cmd.get_args()
        .map(|arg| arg.to_str().unwrap_or(""))
        .collect();
    println!("Debug: Executing: ffmpeg {}", args.join(" "));
}

fn format_timestamp(seconds: f64) -> String {
    let minutes = (seconds / 60.0) as i32;
    let secs = (seconds % 60.0) as i32;
    let centisecs = ((seconds % 1.0) * 100.0) as i32;
    format!("{:02}:{:02}.{:02}", minutes, secs, centisecs)
}

// Add the grouping function
fn group_black_frames(frames: &[(f64, f64)], max_gap: f64) -> Vec<(f64, f64)> {
    // Filter out insignificant black frames first
    let significant_frames: Vec<_> = frames.iter()
        .filter(|&&(start, end)| (end - start) >= MIN_SIGNIFICANT_BLACK)
        .copied()
        .collect();
    
    println!("DEBUG: Found {} significant black frames out of {}", 
        significant_frames.len(), frames.len());

    let mut grouped = Vec::new();
    let mut current_group: Option<(f64, f64)> = None;

    for &(start, end) in &significant_frames {
        match current_group {
            Some((group_start, group_end)) => {
                // Add debug output for groups
                println!("\nDEBUG: Analyzing potential group:");
                println!("  Current group: {} to {}", 
                    format_timestamp(group_start), 
                    format_timestamp(group_end));
                println!("  Next frame: {} to {}", 
                    format_timestamp(start), 
                    format_timestamp(end));
                println!("  Gap: {:.3}s", start - group_end);
                
                if start - group_end <= max_gap {
                    // Extend current group
                    current_group = Some((group_start, end));
                    println!("  -> Extended group to: {} to {}", 
                        format_timestamp(group_start), 
                        format_timestamp(end));
                } else {
                    // Start new group
                    grouped.push((group_start, group_end));
                    current_group = Some((start, end));
                    println!("  -> Started new group");
                }
            }
            None => {
                current_group = Some((start, end));
                println!("\nDEBUG: Started first group: {} to {}", 
                    format_timestamp(start), 
                    format_timestamp(end));
            }
        }
    }

    // Don't forget the last group
    if let Some(group) = current_group {
        grouped.push(group);
    }

    println!("\nDEBUG: Grouping summary:");
    println!("  Input frames: {}", frames.len());
    println!("  Output groups: {}", grouped.len());
    for (i, (start, end)) in grouped.iter().enumerate() {
        println!("  Group {}: {} to {} (duration: {:.3}s)", 
            i + 1,
            format_timestamp(*start), 
            format_timestamp(*end),
            end - start);
    }

    grouped
} 

pub fn detect_black_frames(
    input: &Path,
    config: &BlackFrameConfig,
) -> Result<Vec<(f64, f64)>> {
    let mut cmd = Command::new("ffmpeg");
    cmd.args(&[
        "-i", input.to_str().unwrap(),
        "-vf", &format!(
            "blackdetect=d={}:pic_th={}:pix_th={},select='not(mod(n,{}))'",
            config.min_duration,
            config.max_brightness,
            config.noise_tolerance,
            config.frame_skip
        ),
        "-an",
        "-f", "null",
        "-progress", "pipe:1",
        "-"
    ]);

    let mut child = cmd
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .context("Failed to start FFmpeg")?;

    let stderr = child.stderr.take().unwrap();
    let stderr_reader = std::io::BufReader::new(stderr);
    
    let mut black_frames = Vec::new();
    
    for line in stderr_reader.lines() {
        if let Ok(line) = line {
            if line.contains("blackdetect") {
                if let Some((start, end)) = parse_black_frame_line(&line) {
                    black_frames.push((start, end));
                }
            }
        }
    }

    Ok(black_frames)
} 

pub fn detect_scene_changes(
    input: &Path,
    config: &SceneChangeConfig,
) -> Result<Vec<f64>> {
    let mut cmd = Command::new("ffmpeg");
    cmd.args(&[
        "-i", input.to_str().unwrap(),
        "-vf", &format!(
            "select='gt(scene,{})',metadata=print:file=-",
            config.threshold
        ),
        "-f", "null",
        "-"
    ]);

    let output = cmd
        .output()
        .context("Failed to detect scene changes")?;

    // Parse scene change timestamps from FFmpeg output
    let mut scenes = Vec::new();
    for line in String::from_utf8_lossy(&output.stderr).lines() {
        if line.contains("pts_time:") {
            if let Some(time) = line
                .split("pts_time:")
                .nth(1)
                .and_then(|s| s.trim().parse::<f64>().ok())
            {
                if scenes.last().map_or(true, |&last| time - last >= config.min_scene_length) {
                    scenes.push(time);
                }
            }
        }
    }

    Ok(scenes)
} 

// Add function to score potential boundaries
fn score_segment_boundaries(
    black_frames: &[(f64, f64)],
    scene_changes: &[f64],
    max_gap: f64,
) -> Vec<SegmentBoundary> {
    let mut boundaries = Vec::new();
    
    // Score black frames
    for &(start, end) in black_frames {
        let mut boundary = SegmentBoundary::new(start);
        boundary.black_frame_score = (end - start).min(1.0);  // Normalize to 0-1
        boundaries.push(boundary);
    }

    // Score scene changes
    for &time in scene_changes {
        if let Some(boundary) = boundaries.iter_mut()
            .find(|b| (b.time - time).abs() < max_gap)
        {
            boundary.scene_change_score = 1.0;
        } else {
            let mut boundary = SegmentBoundary::new(time);
            boundary.scene_change_score = 1.0;
            boundaries.push(boundary);
        }
    }

    // Calculate final scores
    for boundary in &mut boundaries {
        boundary.update_scores();
    }

    // Sort by time and filter low scores
    boundaries.sort_by(|a, b| a.time.partial_cmp(&b.time).unwrap());
    boundaries.into_iter()
        .filter(|b| b.total_score > 0.5)
        .collect()
} 