use rusqlite::{Connection, params};
use anyhow::{Context, Result};
use std::path::Path;
use crate::SegmentFingerprint;

pub struct FingerprintDb {
    conn: Connection,
}

impl FingerprintDb {
    pub fn new(db_path: &Path) -> Result<Self> {
        let conn = Connection::open(db_path)
            .with_context(|| format!("Failed to open database at {:?}", db_path))?;
        
        // Create tables if they don't exist
        conn.execute(
            "CREATE TABLE IF NOT EXISTS fingerprints (
                id INTEGER PRIMARY KEY,
                segment_type TEXT NOT NULL,
                duration REAL NOT NULL,
                audio_hash INTEGER NOT NULL,
                video_hash INTEGER NOT NULL,
                first_seen DATETIME DEFAULT CURRENT_TIMESTAMP,
                last_seen DATETIME DEFAULT CURRENT_TIMESTAMP,
                occurrence_count INTEGER DEFAULT 1
            )",
            [],
        )?;

        Ok(Self { conn })
    }

    pub fn find_similar_fingerprint(&self, fingerprint: &SegmentFingerprint, similarity_threshold: f64) -> Result<Option<i64>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, audio_hash, video_hash, duration 
             FROM fingerprints 
             WHERE ABS(duration - ?) < 0.5"
        )?;
        
        let rows = stmt.query_map(
            params![fingerprint.duration],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                    row.get::<_, f64>(3)?,
                ))
            },
        )?;

        let mut best_match: Option<(i64, f64)> = None;

        for row in rows {
            let (id, stored_audio, stored_video, stored_duration) = row?;
            let similarity = calculate_similarity(
                fingerprint.audio_hash,
                fingerprint.video_hash,
                fingerprint.duration,
                stored_audio as u64,
                stored_video as u64,
                stored_duration,
            );

            if similarity >= similarity_threshold {
                match best_match {
                    None => best_match = Some((id, similarity)),
                    Some((_, best_sim)) if similarity > best_sim => {
                        best_match = Some((id, similarity))
                    }
                    _ => {}
                }
            }
        }

        Ok(best_match.map(|(id, _)| id))
    }

    pub fn store_fingerprint(&self, fingerprint: &SegmentFingerprint, segment_type: &str) -> Result<i64> {
        self.conn.execute(
            "INSERT INTO fingerprints (segment_type, duration, audio_hash, video_hash)
             VALUES (?, ?, ?, ?)",
            params![
                segment_type,
                fingerprint.duration,
                fingerprint.audio_hash as i64,
                fingerprint.video_hash as i64,
            ],
        )?;

        Ok(self.conn.last_insert_rowid())
    }

    pub fn update_fingerprint_occurrence(&self, id: i64) -> Result<()> {
        self.conn.execute(
            "UPDATE fingerprints 
             SET occurrence_count = occurrence_count + 1,
                 last_seen = CURRENT_TIMESTAMP
             WHERE id = ?",
            params![id],
        )?;

        Ok(())
    }
}

fn calculate_similarity(
    audio_hash1: u64,
    video_hash1: u64,
    duration1: f64,
    audio_hash2: u64,
    video_hash2: u64,
    duration2: f64,
) -> f64 {
    // Hamming distance for hashes
    let audio_distance = (audio_hash1 ^ audio_hash2).count_ones() as f64 / 64.0;
    let video_distance = (video_hash1 ^ video_hash2).count_ones() as f64 / 64.0;
    
    // Duration similarity
    let duration_diff = (duration1 - duration2).abs() / duration1.max(duration2);
    
    // Weighted combination
    let audio_weight = 0.4;
    let video_weight = 0.4;
    let duration_weight = 0.2;
    
    1.0 - (
        audio_weight * audio_distance +
        video_weight * video_distance +
        duration_weight * duration_diff
    )
} 