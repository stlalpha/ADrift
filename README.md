# ADrift

ADrift is a Commercial Advertisement Archival Extraction Tool for digitized broadcast content, particularly from the 1980s and 1990s. It identifies commercial segments and station identifications by analyzing black frame patterns and standard broadcast timing patterns common in television of that era.

## Current Capabilities

- Detects and extracts:
  - Commercial segments (15, 30, or 60 seconds)
  - Station IDs (3, 5, or 10 seconds)
- Uses black frame analysis common in broadcast content
- Supports multiple input and output formats (MOV, MP4, WebM, MKV)
- Configurable detection parameters for different source materials
- Real-time progress tracking and segment identification

## Why Preserve These Segments?

### Commercials
Vintage commercials capture unique historical content:
- Regional businesses and their stories
- Local personalities and cultural figures
- Period-specific pricing and products
- Evolution of advertising techniques
- Cultural attitudes and social norms
- It's pretty fun

### Station IDs
Station identifications are equally valuable historical artifacts:
- Local broadcast branding and identity
- Network affiliate relationships
- Regional broadcast history
- Technical broadcast standards
- Station call signs and channel numbers
- Often featuring local landmarks or cultural elements

## Prerequisites

- Rust (latest stable version)
- FFmpeg version 4.x installed on your system
  - Note: Currently requires FFmpeg 4.x specifically due to filter compatibility
  - On macOS: `brew install ffmpeg@4`
  - On Linux: Check your package manager for ffmpeg-4

## Installation

```bash
# Clone the repository
git clone https://github.com/stlalpha/adrift.git
cd adrift

# Build the project
cargo build --release

# Optional: Install globally
cargo install --path .
```

## Usage

Note: ADrift will check for FFmpeg 4.x compatibility at runtime and warn if an incompatible version is detected.

Basic usage:
```bash
adrift input_video.webm ./output_directory
```

With options:
```bash
adrift input_video.webm ./output_directory \
    --output-format mov \
    --black-threshold 0.1 \
    --min-black-frames 3
```

### Options

- `--output-format <FORMAT>`: Output format for extracted commercials (default: same as input)
  - Supported formats: `same`, `mp4`, `webm`, `mkv`, `mov`
- `--black-threshold <FLOAT>`: Threshold for black frame detection (0.0-1.0, default: 0.1)
- `--min-black-frames <INT>`: Minimum number of black frames for detection (default: 3)

## Contributing

Contributions are welcome! Please feel free to submit a Pull Request.

## License

This project is licensed under the MIT License - see the [LICENSE](LICENSE) file for details.

## Author

Jim McBride ([@stlalpha](https://github.com/stlalpha))

