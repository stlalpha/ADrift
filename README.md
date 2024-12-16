# ADrift

ADrift is a Commercial Advertisement Archival Extraction Tool for digitized broadcast content, particularly from the 1980s and 1990s. It identifies commercial segments by analyzing black frame patterns and standard commercial lengths common in broadcast television of that era.

## Current Capabilities

- Detects commercial segments using:
  - Black frame analysis (common between commercials in broadcast content)
  - Standard commercial length matching (15, 30, or 60 seconds)
- Supports multiple input and output formats (MOV, MP4, WebM, MKV)
- Configurable detection parameters for different source materials

## Use Cases

- Extracting commercials from digitized broadcast content
- Preserving regional/local advertising content
- Processing personal or institutional video collections


## Why Preserve Commercials?

Vintage commercials, especially from local broadcasts, capture unique historical content:

- Regional businesses and their stories
- Local personalities and cultural figures
- Period-specific pricing and products
- Evolution of advertising techniques
- Cultural attitudes and social norms
- It's pretty fun

## Prerequisites

- Rust (latest stable version)
- FFmpeg installed on your system

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

