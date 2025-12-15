use clap::Parser;
use crossterm::terminal;
use image::{imageops::FilterType, GenericImageView};
use std::path::PathBuf;

/// A CLI tool to display images in the terminal using 
/// Legacy Computing 2x3 Sextant characters (Unicode 13.0)
/// Uses Block Truncation Coding (BTC) for true-color structure.
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Paths to the input images
    #[arg(value_name = "FILES", num_args = 1..)]
    paths: Vec<PathBuf>,

    /// Enable edge detection enhancement (Sharpening)
    #[arg(short, long)]
    edges: bool,

    /// Specify the width of the output in characters
    /// If not provided, the terminal width will be used.
    #[arg(short, long)]
    width: Option<u32>,
}

// Map 0-63 bitmask to Unicode Sextants
// Bit order: TL(1), TR(2), ML(4), MR(8), BL(16), BR(32)
const SEXTANTS: [char; 64] = [
    ' ', '🬀', '🬁', '🬂', '🬃', '🬄', '🬅', '🬆', '🬇', '🬈', '🬉', '🬊', '🬋', '🬌', '🬍', '🬎',
    '🬏', '🬐', '🬑', '🬒', '🬓', '▌', '🬔', '🬕', '🬖', '🬗', '🬘', '🬙', '🬚', '🬛', '🬜', '🬝',
    '🬞', '🬟', '🬠', '🬡', '🬢', '🬣', '🬤', '🬥', '🬦', '🬧', '▐', '🬨', '🬩', '🬪', '🬫', '🬬',
    '🬭', '🬮', '🬯', '🬰', '🬱', '🬲', '🬳', '🬴', '🬵', '🬶', '🬷', '🬸', '🬹', '🬺', '🬻', '█',
];

#[derive(Clone, Copy)]
struct PixelData {
    luma: f32,
    r: f32, 
    g: f32,
    b: f32,
    mask_bit: usize,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    if args.paths.is_empty() {
        eprintln!("No input files specified.");
        return Ok(());
    }

    for path in &args.paths {
        if args.paths.len() > 1 {
            println!("\n--- {} ---", path.display());
        }

        match render_image(path, &args) {
            Ok(_) => {},
            Err(e) => eprintln!("Error processing {}: {}", path.display(), e),
        }
    }

    Ok(())
}

fn render_image(path: &PathBuf, args: &Args) -> Result<(), Box<dyn std::error::Error>> {
    let img = image::open(path).map_err(|e| format!("Failed to open image: {}", e))?;
    let (term_w, term_h) = terminal::size().unwrap_or((80, 24));

    let mut target_width = (term_w as u32) * 2;
    let target_height = ((term_h as u32).saturating_sub(2)) * 4;

    if args.width.is_some() {
        target_width = args.width.unwrap() * 2;
    }

    // Resize and convert to RGBA8 immediately
    let mut resized = img.resize(target_width, target_height, FilterType::Lanczos3).to_rgba8();

    let mut resized = image::imageops::resize(
        &resized,
        resized.width(),
        (resized.height() as f32 * 3.0/4.0) as u32,
        FilterType::Lanczos3,
    );
    if args.edges {
        let kernel = [
            0.0, -1.0, 0.0,
            -1.0, 5.0, -1.0,
            0.0, -1.0, 0.0
        ];
        resized = image::imageops::filter3x3(&resized, &kernel);
    }

    let (width, height) = resized.dimensions();

    for y in (0..height).step_by(3) {
        let mut line = String::new();
        
        for x in (0..width).step_by(2) {
            
            let mut pixels: Vec<PixelData> = Vec::with_capacity(6);
            let mut luma_sum = 0.0;

            // (0,0)->1, (1,0)->2, (0,1)->4, (1,1)->8, (0,2)->16, (1,2)->32
            let coords = [
                (0, 0, 1),  (1, 0, 2),
                (0, 1, 4),  (1, 1, 8),
                (0, 2, 16), (1, 2, 32),
            ];

            // 1. Collect pixels and calculate Average Luma
            for (dx, dy, bit) in coords {
                if x + dx < width && y + dy < height {
                    let p = resized.get_pixel(x + dx, y + dy);
                    let r = p[0] as f32 / 255.0;
                    let g = p[1] as f32 / 255.0;
                    let b = p[2] as f32 / 255.0;
                    
                    // Rec. 709 Luma
                    let luma = 0.2126 * r + 0.7152 * g + 0.0722 * b;
                    luma_sum += luma;
                    
                    pixels.push(PixelData { luma, r, g, b, mask_bit: bit });
                }
            }

            if pixels.is_empty() {
                line.push(' ');
                continue;
            }

            let luma_threshold = luma_sum / pixels.len() as f32;

            // 2. Separate into Foreground (>= threshold) and Background (< threshold)
            let mut fg_group = Vec::with_capacity(6);
            let mut bg_group = Vec::with_capacity(6);
            let mut char_mask = 0;

            for p in pixels {
                // If luma is greater than average, it's foreground
                if p.luma >= luma_threshold {
                    fg_group.push(p);
                    char_mask |= p.mask_bit;
                } else {
                    bg_group.push(p);
                }
            }

            // Edge case: if all pixels are identical, they all land in FG (>= threshold).
            // This results in a full block (█). This is visually correct.
            // Edge case: if mask is 0 (all dark), space is printed with BG color.

            let (fg_r, fg_g, fg_b) = average_color(&fg_group);
            let (bg_r, bg_g, bg_b) = average_color(&bg_group);

            let sextant_char = SEXTANTS.get(char_mask).unwrap_or(&' ');

            use std::fmt::Write as _;
            write!(line, "\x1b[38;2;{};{};{};48;2;{};{};{}m{}", 
                fg_r, fg_g, fg_b,
                bg_r, bg_g, bg_b,
                sextant_char
            ).unwrap();
        }
        println!("{}\x1b[0m", line);
    }
    Ok(())
}

/// Averages RGB colors using Linear space
fn average_color(pixels: &[PixelData]) -> (u8, u8, u8) {
    if pixels.is_empty() {
        // If a group is empty, its color doesn't matter visually 
        // (e.g., if no FG pixels, char is ' ', so FG color is invisible).
        // We return black to be safe.
        return (0, 0, 0);
    }

    let mut r_sum = 0.0;
    let mut g_sum = 0.0;
    let mut b_sum = 0.0;
    let count = pixels.len() as f32;

    for p in pixels {
        r_sum += srgb_to_linear(p.r);
        g_sum += srgb_to_linear(p.g);
        b_sum += srgb_to_linear(p.b);
    }

    let r_avg = r_sum / count;
    let g_avg = g_sum / count;
    let b_avg = b_sum / count;

    let r_u8 = (linear_to_srgb(r_avg).clamp(0.0, 1.0) * 255.0).round() as u8;
    let g_u8 = (linear_to_srgb(g_avg).clamp(0.0, 1.0) * 255.0).round() as u8;
    let b_u8 = (linear_to_srgb(b_avg).clamp(0.0, 1.0) * 255.0).round() as u8;

    (r_u8, g_u8, b_u8)
}

fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

fn linear_to_srgb(c: f32) -> f32 {
    if c <= 0.0031308 { 12.92 * c } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 }
}