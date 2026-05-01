use std::path::Path;

use regex::Regex;
use walkdir::WalkDir;

pub const VIDEO_EXTENSIONS: &[&str] = &[
    "mkv", "mp4", "m4v", "mov", "avi", "wmv", "flv", "webm", "ts", "m2ts", "mts", "ogv", "ogm",
    "vob", "3gp", "rm", "rmvb", "mpg", "mpeg",
];

#[derive(Debug, Clone)]
pub struct DetectionRule {
    pub detection_regex: String,
    pub title_regex: String,
}

#[derive(Debug, Clone)]
pub struct ScannedEpisode {
    pub path: String,
    pub relative_path: String,
    pub file_name: String,
    pub file_type: String,
    pub title: String,
    pub title_key: String,
    pub episode_number: Option<f64>,
    pub size: i64,
}

#[derive(Debug, Clone)]
pub struct UnmatchedFile {
    pub path: String,
    pub relative_path: String,
    pub file_name: String,
    pub reason: String,
}

#[derive(Debug, Default)]
pub struct ScanRootResult {
    pub episodes: Vec<ScannedEpisode>,
    pub unmatched: Vec<UnmatchedFile>,
}

struct CompiledRule {
    detection: Regex,
    title: Regex,
}

pub fn is_video_file(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| {
            let lower = ext.to_ascii_lowercase();
            VIDEO_EXTENSIONS.iter().any(|allowed| *allowed == lower)
        })
        .unwrap_or(false)
}

pub fn scan_root(root: &Path, rules: &[DetectionRule]) -> Result<ScanRootResult, String> {
    if !root.exists() {
        return Err(format!("Folder does not exist: {}", root.display()));
    }
    if !root.is_dir() {
        return Err(format!("Path is not a directory: {}", root.display()));
    }

    let compiled = compile_rules(rules)?;
    let mut result = ScanRootResult::default();

    for entry in WalkDir::new(root)
        .follow_links(false)
        .into_iter()
        .filter_map(Result::ok)
    {
        let path = entry.path();
        if !entry.file_type().is_file() || !is_video_file(path) {
            continue;
        }

        let file_name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_string();
        let relative_path = path
            .strip_prefix(root)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/");
        let absolute_path = path.to_string_lossy().to_string();
        let size = entry.metadata().map(|m| m.len() as i64).unwrap_or(0);

        match detect_episode(path, &file_name, &relative_path, size, &compiled) {
            Some(episode) => result.episodes.push(episode),
            None => result.unmatched.push(UnmatchedFile {
                path: absolute_path,
                relative_path,
                file_name,
                reason: "No enabled regex rule matched this filename.".to_string(),
            }),
        }
    }

    result.episodes.sort_by(|a, b| {
        a.relative_path
            .to_lowercase()
            .cmp(&b.relative_path.to_lowercase())
    });
    result.unmatched.sort_by(|a, b| {
        a.relative_path
            .to_lowercase()
            .cmp(&b.relative_path.to_lowercase())
    });
    Ok(result)
}

fn compile_rules(rules: &[DetectionRule]) -> Result<Vec<CompiledRule>, String> {
    rules
        .iter()
        .map(|rule| {
            let detection = Regex::new(&rule.detection_regex)
                .map_err(|e| format!("invalid detection regex {:?}: {e}", rule.detection_regex))?;
            let title = Regex::new(&rule.title_regex)
                .map_err(|e| format!("invalid title regex {:?}: {e}", rule.title_regex))?;
            Ok(CompiledRule { detection, title })
        })
        .collect()
}

fn detect_episode(
    path: &Path,
    file_name: &str,
    relative_path: &str,
    size: i64,
    rules: &[CompiledRule],
) -> Option<ScannedEpisode> {
    for rule in rules {
        if !rule.detection.is_match(file_name) {
            continue;
        }
        let Some(caps) = rule.title.captures(file_name) else {
            continue;
        };
        let title = caps
            .name("title")
            .or_else(|| caps.get(1))
            .map(|m| clean_title(m.as_str()))
            .filter(|s| !s.is_empty())?;
        let episode_number = caps
            .name("episode")
            .or_else(|| caps.get(2))
            .and_then(|m| m.as_str().parse::<f64>().ok());
        let file_type = path
            .extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase();

        return Some(ScannedEpisode {
            path: path.to_string_lossy().to_string(),
            relative_path: relative_path.to_string(),
            file_name: file_name.to_string(),
            file_type,
            title_key: title_key(&title),
            title,
            episode_number,
            size,
        });
    }

    None
}

pub fn title_key(title: &str) -> String {
    title
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_ascii_lowercase()
}

fn clean_title(raw: &str) -> String {
    raw.replace('_', " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}
