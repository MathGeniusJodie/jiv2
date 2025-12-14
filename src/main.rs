use clap::{Parser, error};
use crossterm::terminal;
use image::{imageops::FilterType, GenericImageView, Pixel};
use std::path::PathBuf;

/// A CLI tool to display images in the terminal using Braille characters
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Path to the input image
    #[arg(value_name = "FILE")]
    path: PathBuf,

    /// Enable edge detection enhancement
    #[arg(short, long)]
    edges: bool,

    /// Invert colors (useful for dark terminal themes)
    #[arg(short, long)]
    invert: bool,

    /// Threshold for binary conversion (0-255). Lower = more dots.
    #[arg(short, long, default_value_t = 128)]
    threshold: u8,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
// --- Color space conversion functions ---
fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

fn linear_to_srgb(c: f32) -> f32 {
    if c <= 0.0031308 {
        12.92 * c
    } else {
        1.055 * c.powf(1.0 / 2.4) - 0.055
    }
}

fn srgb_to_oklab(r: f32, g: f32, b: f32) -> (f32, f32, f32) {
    // Convert sRGB [0,1] to linear
    let r_lin = srgb_to_linear(r);
    let g_lin = srgb_to_linear(g);
    let b_lin = srgb_to_linear(b);
    linear_to_oklab(r_lin, g_lin, b_lin)
}

fn oklab_to_srgb(l: f32, a: f32, b: f32) -> (f32, f32, f32) {
    let (r_lin, g_lin, b_lin) = oklab_to_linear(l, a, b);
    (
        linear_to_srgb(r_lin),
        linear_to_srgb(g_lin),
        linear_to_srgb(b_lin),
    )
}

fn linear_to_oklab(r: f32, g: f32, b: f32) -> (f32, f32, f32) {
    // 1. Linear RGB to LMS
    let l = 0.4122214708 * r + 0.5363325363 * g + 0.0514459929 * b;
    let m = 0.2119034982 * r + 0.6806995451 * g + 0.1073969566 * b;
    let s = 0.0883024619 * r + 0.2817188376 * g + 0.6299787005 * b;
    // 2. Cube root
    let l_ = l.cbrt();
    let m_ = m.cbrt();
    let s_ = s.cbrt();
    // 3. LMS to Oklab
    let l = 0.2104542553 * l_ + 0.7936177850 * m_ - 0.0040720468 * s_;
    let a = 1.9779984951 * l_ - 2.4285922050 * m_ + 0.4505937099 * s_;
    let b = 0.0259040371 * l_ + 0.7827717662 * m_ - 0.8086757660 * s_;
    (l, a, b)
}

fn oklab_to_linear(l: f32, a: f32, b: f32) -> (f32, f32, f32) {
    // 1. Oklab to LMS
    let l_ = l + 0.3963377774 * a + 0.2158037573 * b;
    let m_ = l - 0.1055613458 * a - 0.0638541728 * b;
    let s_ = l - 0.0894841775 * a - 1.2914855480 * b;
    // 2. Cube
    let l = l_.powi(3);
    let m = m_.powi(3);
    let s = s_.powi(3);
    // 3. LMS to linear RGB
    let r = 4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s;
    let g = -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s;
    let b = -0.0041960863 * l - 0.7034186147 * m + 1.7076147010 * s;
    (r, g, b)
}
// --- End color space conversion functions ---

/// Raises the luma (L) of an Oklab color as high as possible while remaining a valid sRGB color.
/// Returns the new (L, a, b) tuple.
fn maximize_oklab_luma_within_srgb(l: f32, a: f32, b: f32) -> (f32, f32, f32) {
    // Binary search for the highest L such that oklab_to_srgb(L, a, b) is in [0,1] for all channels
    let mut low = l;
    let mut high = 1.0;
    let mut best = l;
    for _ in 0..20 {
        let mid = (low + high) * 0.5;
        let (r, g, b_) = oklab_to_srgb(mid, a, b);
        if r >= 0.0 && r <= 1.0 && g >= 0.0 && g <= 1.0 && b_ >= 0.0 && b_ <= 1.0 {
            best = mid;
            low = mid;
        } else {
            high = mid;
        }
    }
    (best, a, b)
}
    let args = Args::parse();

    // 1. Load the image
    let img = image::open(&args.path).map_err(|e| format!("Failed to open image: {}", e))?;

    // 2. Get terminal size
    let (term_w, term_h) = terminal::size().unwrap_or((80, 24));

    // 3. Calculate target size
    // Braille characters are 2 pixels wide and 4 pixels tall.
    // We multiply terminal dimensions by these factors to get "virtual pixel" resolution.
    // We subtract 1 from height to leave room for the prompt line.
    let target_width = (term_w as u32) * 2;
    let target_height = ((term_h as u32).saturating_sub(1)) * 4;

    // 4. Resize image preserving aspect ratio
    // We use Lanczos3 for high-quality downscaling
    let mut resized = img.resize(target_width, target_height, FilterType::Lanczos3);

    // 5. Convert to Grayscale (Luma8)
    let mut gray_image = resized.to_luma8();

    // 6. Optional: Edge Detection
    if args.edges {
        // Edge enhancement kernel (sharpening)
        //  0 -1  0
        // -1  5 -1
        //  0 -1  0
        let kernel = [
            0.0, -1.0, 0.0,
            -1.0, 5.0, -1.0,
            0.0, -1.0, 0.0
        ];
        gray_image = image::imageops::filter3x3(&gray_image, &kernel);
    }

    // 7. Render Loop
    let (width, height) = gray_image.dimensions();

    let mut error_diffusion : Vec<Vec<f32>> = vec![vec![0.0; width as usize]; height as usize];

    // Iterate through the image in chunks of 2x4 (Width x Height of a Braille char)
    for y in (0..height).step_by(4) {
        let mut line = String::new();
        
        for x in (0..width).step_by(2) {
            // get average color for the 2x4 block
            let mut r_total: f32 = 0.0;
            let mut g_total: f32 = 0.0;
            let mut b_total: f32 = 0.0;
            for dy in 0..4 {
                for dx in 0..2 {
                    if x + dx < width && y + dy < height {
                        let pixel = resized.get_pixel(x + dx, y + dy);
                        r_total += srgb_to_linear(pixel[0] as f32 / 255.0);
                        g_total += srgb_to_linear(pixel[1] as f32 / 255.0);
                        b_total += srgb_to_linear(pixel[2] as f32 / 255.0);
                    }
                }
            }
            let count = 8.0; // 2*4
            let r_avg= r_total / count;
            let g_avg = g_total / count;
            let b_avg = b_total / count;

            // Convert to 0-255 sRGB for ANSI
            let r_ansi = (r_avg.sqrt().clamp(0.0, 1.0) * 255.0).round() as u8;
            let g_ansi = (g_avg.sqrt().clamp(0.0, 1.0) * 255.0).round() as u8;
            let b_ansi = (b_avg.sqrt().clamp(0.0, 1.0) * 255.0).round() as u8;

            let mut byte_mask: u8 = 0;

            // Define Braille dot coordinates relative to (x, y)
            // Braille Unicode Pattern:
            // (0,0) -> 0x01   (1,0) -> 0x08
            // (0,1) -> 0x02   (1,1) -> 0x10
            // (0,2) -> 0x04   (1,2) -> 0x20
            // (0,3) -> 0x40   (1,3) -> 0x80
            
            let coords = [
                (0, 0, 0x01), (0, 1, 0x02), (0, 2, 0x04), (1, 0, 0x08),
                (1, 1, 0x10), (1, 2, 0x20), (0, 3, 0x40), (1, 3, 0x80),
            ];

            for (dx, dy, bit) in coords {
                if x + dx < width && y + dy < height {
                    let pixel = gray_image.get_pixel(x + dx, y + dy);
                    let luma = (pixel.0[0] as f32 / 255.0).sqrt() * 255.0 + error_diffusion[(y + dy) as usize][(x + dx) as usize];
                    // adjust using l_diff
                    //let luma = (luma as f32 - (1.0-l_avg) * 255.0).clamp(0.0, 255.0) as u8;

                    let is_on = if args.invert {
                        luma < args.threshold as f32
                    } else {
                        luma > args.threshold as f32
                    };

                    if is_on {
                        byte_mask |= bit;
                    }
                    // Error diffusion
                    let error_value = luma as i16 - if is_on { 255 } else { 0 };
                    // Distribute error to neighboring pixels
                    let diffusion_coords = [
                        // Extended error diffusion kernel (Stucki)
                        (1, 0, 8.0 / 42.0),
                        (2, 0, 4.0 / 42.0),
                        (-2, 1, 2.0 / 42.0),
                        (-1, 1, 4.0 / 42.0),
                        (0, 1, 8.0 / 42.0),
                        (1, 1, 4.0 / 42.0),
                        (2, 1, 2.0 / 42.0),
                        (-2, 2, 1.0 / 42.0),
                        (-1, 2, 2.0 / 42.0),
                        (0, 2, 4.0 / 42.0),
                        (1, 2, 2.0 / 42.0),
                        (2, 2, 1.0 / 42.0),
                    ];
                    for (dx_e, dy_e, factor) in diffusion_coords {
                        let nx = x as i32 + dx as i32 + dx_e;
                        let ny = y as i32 + dy as i32 + dy_e;
                        if nx >= 0 && nx < width as i32 && ny >= 0 && ny < height as i32 {
                            error_diffusion[ny as usize][nx as usize] += error_value as f32 * factor;
                        }
                    }
                }
            }

            // Base Braille Unicode char is U+2800
            let braille_char = char::from_u32(0x2800 + byte_mask as u32).unwrap_or(' ');
            // ANSI escape: bold + truecolor foreground
            use std::fmt::Write as _;
            write!(line, "\x1b[1;38;2;{};{};{}m{}\x1b[0m", r_ansi, g_ansi, b_ansi, braille_char).unwrap();
        }
        println!("{}", line);
    }

    Ok(())
}