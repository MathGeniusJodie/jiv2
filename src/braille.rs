use clap::Parser;
use crossterm::terminal;
use image::{imageops::FilterType, GenericImageView};
use std::path::PathBuf;

/// A CLI tool to display images in the terminal using Braille characters
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Paths to the input images
    #[arg(value_name = "FILES", num_args = 1..)]
    paths: Vec<PathBuf>,

    /// Enable edge detection enhancement
    #[arg(short, long)]
    edges: bool,

    // optional width parameter
    // Specify the width of the output in characters
    // If not provided, the terminal width will be used.
    // this value is in characters, not pixels
    #[arg(short, long)]
    width: Option<u32>,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();

    if args.paths.is_empty() {
        eprintln!("No input files specified.");
        return Ok(());
    }

    for path in &args.paths {
        // Print filename header if there are multiple files
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
    // 1. Load the image
    let img = image::open(path).map_err(|e| format!("Failed to open image: {}", e))?;

    // 2. Get terminal size
    let (term_w, term_h) = terminal::size().unwrap_or((80, 24));

    // 3. Calculate target size
    // Braille characters are 2 pixels wide and 4 pixels tall.
    let mut target_width = (term_w as u32) * 2;
    // We subtract a bit from height to ensure it fits with prompt/headers
    let target_height = ((term_h as u32).saturating_sub(2)) * 4;

    if args.width.is_some() {
        target_width = args.width.unwrap() * 2;
    }

    // 4. Resize image preserving aspect ratio
    let resized = img.resize(target_width, target_height, FilterType::Lanczos3);

    // 5. Convert to Grayscale (Luma8) for structure
    let mut gray_image = resized.to_luma8();

    // 6. Optional: Edge Detection
    if args.edges {
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
            // Variables to track color averages and luma range
            let mut r_total: f32 = 0.0;
            let mut g_total: f32 = 0.0;
            let mut b_total: f32 = 0.0;
            
            let mut min_luma: u8 = 255;
            let mut max_luma: u8 = 0;

            for dy in 0..4 {
                for dx in 0..2 {
                    if x + dx < width && y + dy < height {
                        // Color accumulation
                        let pixel = resized.get_pixel(x + dx, y + dy);
                        r_total += srgb_to_linear(pixel[0] as f32 / 255.0);
                        g_total += srgb_to_linear(pixel[1] as f32 / 255.0);
                        b_total += srgb_to_linear(pixel[2] as f32 / 255.0);

                        // Luma min/max tracking from the grayscale image
                        let luma_pixel = gray_image.get_pixel(x + dx, y + dy)[0];
                        if luma_pixel < min_luma { min_luma = luma_pixel; }
                        if luma_pixel > max_luma { max_luma = luma_pixel; }
                    }
                }
            }
            
            let count = 8.0; // 2*4
            let r_avg= r_total / count;
            let g_avg = g_total / count;
            let b_avg = b_total / count;

            // Check if the block is relatively flat (solid color or smooth gradient)
            let diff = max_luma.abs_diff(min_luma);
            let is_flat = diff < 20; // Threshold: adjusted to 30 for smoothness

            let (r_final, g_final, b_final);
            let mut byte_mask: u8 = 0;

            let diff = max_luma.abs_diff(min_luma) as f32 / 255.0;
            let bleh = 0.8 * (1.0 - diff) + 0.5 * diff;
            //let bleh = 0.5 * (1.0 - diff) + 0.5 * diff;
            let blah = 1.0-bleh;


            
                // RENDER DITHERED BLOCK (Original Logic)

                // 1. Color: Apply sqrt boost for sparse dots
                let (r, g, b) = (r_avg.powf(bleh), g_avg.powf(bleh), b_avg.powf(bleh));
                r_final = linear_to_srgb(r);
                g_final = linear_to_srgb(g);
                b_final = linear_to_srgb(b);

                // 2. Shape: Calculate Braille dots via error diffusion
                let coords = [
                    (0, 0, 0x01), (0, 1, 0x02), (0, 2, 0x04), (1, 0, 0x08),
                    (1, 1, 0x10), (1, 2, 0x20), (0, 3, 0x40), (1, 3, 0x80),
                ];

                for (dx, dy, bit) in coords {
                    if x + dx < width && y + dy < height {
                        let pixel = gray_image.get_pixel(x + dx, y + dy);
                        // Apply error diffusion from previous pixels
                        let luma = srgb_to_linear(pixel.0[0] as f32 / 255.0).powf(blah) * 255.0 + error_diffusion[(y + dy) as usize][(x + dx) as usize];

                        let is_on = luma > 128.0;

                        if is_on {
                            byte_mask |= bit;
                        }
                        
                        // Calculate Error
                        let error_value = luma as i16 - if is_on { 255 } else { 0 };
                        
                        // Distribute error to neighboring pixels (Stucki kernel)
                        let diffusion_coords = [
                            (1, 0, 8.0 / 42.0), (2, 0, 4.0 / 42.0),
                            (-2, 1, 2.0 / 42.0), (-1, 1, 4.0 / 42.0), (0, 1, 8.0 / 42.0), (1, 1, 4.0 / 42.0), (2, 1, 2.0 / 42.0),
                            (-2, 2, 1.0 / 42.0), (-1, 2, 2.0 / 42.0), (0, 2, 4.0 / 42.0), (1, 2, 2.0 / 42.0), (2, 2, 1.0 / 42.0),
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
            

            // Convert calculated color to 0-255 sRGB for ANSI
            let r_ansi = (r_final.clamp(0.0, 1.0) * 255.0).round() as u8;
            let g_ansi = (g_final.clamp(0.0, 1.0) * 255.0).round() as u8;
            let b_ansi = (b_final.clamp(0.0, 1.0) * 255.0).round() as u8;

            // Base Braille Unicode char is U+2800
            let braille_char = char::from_u32(0x2800 + byte_mask as u32).unwrap_or(' ');
            
            // ANSI escape: bold + truecolor foreground + black background
            use std::fmt::Write as _;
            write!(line, "\x1b[1;38;2;{};{};{};48;2;0;0;0m{}\x1b[0m", r_ansi, g_ansi, b_ansi, braille_char).unwrap();
        }
        println!("{}", line);
    }
    Ok(())
}


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