use clap::Parser;
use crossterm::terminal;
use image::{imageops::FilterType, GenericImageView};
use std::path::PathBuf;

/// A CLI tool to display images in the terminal using 
/// Braille Pattern characters (Unicode 13.0, 2x4 grid).
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

#[derive(Clone, Copy)]
struct PixelData {
    luma: f32,
    r: f32, 
    g: f32,
    b: f32,
    mask_bit: u32,
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

    // Braille is 2x4. 
    let mut target_width = (term_w as u32) * 2 ;
    // We reserve 2 lines for prompt/spacing, multiply by 4 for braille height
    let target_height = ((term_h as u32).saturating_sub(2)) * 4;

    if let Some(w) = args.width {
        target_width = w * 2;
    }

    // Resize to exact dimensions. 
    // Note: We removed the 3/4 aspect ratio adjustment used for sextants.
    // Since terminal cells are roughly 1:2 and Braille is 2x4, the sub-pixels 
    // are naturally square-ish (0.5 w / 0.25 h = 2:1 ratio in a 1:2 cell = 1:1).
    let mut resized = img.resize(target_width, target_height, FilterType::Lanczos3).to_rgba8();

    if args.edges {
        let kernel = [
            0.0, -1.0, 0.0,
            -1.0, 5.0, -1.0,
            0.0, -1.0, 0.0
        ];
        resized = image::imageops::filter3x3(&resized, &kernel);
    }

    let (width, height) = resized.dimensions();
    let mut error_diffusion : Vec<Vec<(f32,f32,f32)>> = vec![vec![(0.0,0.0,0.0); width as usize]; height as usize];

    // Iterate over character blocks (2 wide, 4 high)
    for y in (0..height).step_by(4) {
        let mut line = String::new();
        
        for x in (0..width).step_by(2) {
            
            let mut pixels: Vec<PixelData> = Vec::with_capacity(8);
            let mut luma_sum = 0.0;

            // Unicode Braille Bit mapping:
            // (0,0)->0x1, (0,1)->0x2, (0,2)->0x4, (1,0)->0x8, 
            // (1,1)->0x10, (1,2)->0x20, (0,3)->0x40, (1,3)->0x80
            // Note: The braille dot ordering is unique (1,2,3,7 for left col, 4,5,6,8 for right col)
            let coords = [
                (0, 0, 0x01), (1, 0, 0x08),
                (0, 1, 0x02), (1, 1, 0x10),
                (0, 2, 0x04), (1, 2, 0x20),
                (0, 3, 0x40), (1, 3, 0x80),
            ];

            // 1. Collect pixels and calculate Average Luma
            for (dx, dy, bit) in coords {
                if x + dx < width && y + dy < height {
                    let p = resized.get_pixel(x + dx, y + dy);
                    let r = srgb_to_linear(p[0] as f32 / 255.0) + error_diffusion[(y + dy) as usize][(x + dx) as usize].0;
                    let g = srgb_to_linear(p[1] as f32 / 255.0) + error_diffusion[(y + dy) as usize][(x + dx) as usize].1;
                    let b = srgb_to_linear(p[2] as f32 / 255.0) + error_diffusion[(y + dy) as usize][(x + dx) as usize].2;
                    let r = linear_to_srgb(r);
                    let g = linear_to_srgb(g);
                    let b = linear_to_srgb(b);
                    
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
            let mut fg_group = Vec::with_capacity(8);
            let mut bg_group = Vec::with_capacity(8);
            let mut char_mask = 0;

            for p in pixels {
                if p.luma >= luma_threshold {
                    fg_group.push(p);
                    char_mask |= p.mask_bit;
                } else {
                    bg_group.push(p);
                }
            }

            // Braille Unicode base is 0x2800
            let braille_char = char::from_u32(0x2800 + char_mask).unwrap_or(' ');

            
            let (bg_r, bg_g, bg_b) = average_color_linear(&bg_group).unwrap_or(
                average_color_linear(&fg_group).unwrap_or((0.0,0.0,0.0))
            );
            let (target_fg_r, target_fg_g, target_fg_b) = average_color_linear(&fg_group).unwrap_or((bg_r, bg_g, bg_b));

            // 3. Calculate the "Un-mixed" foreground color
            // Logic: Target_Color = 0.5 * FG_Dot + 0.5 * BG
            // Therefore: FG_Dot = 2 * Target_Color - BG
            let (final_fg_r, final_fg_g, final_fg_b) = solve_dot_color(
                (target_fg_r, target_fg_g, target_fg_b),
                (bg_r, bg_g, bg_b)
            );

            // mix clamped final_fg with bg to get actual rendered color
            let mixed_r = 0.5 * final_fg_r.clamp(0.0, 1.0) + 0.5 * bg_r;
            let mixed_g = 0.5 * final_fg_g.clamp(0.0, 1.0) + 0.5 * bg_g;
            let mixed_b = 0.5 * final_fg_b.clamp(0.0, 1.0) + 0.5 * bg_b;
            // 4. Calculate error and distribute using Stucki dithering
            let err_r = target_fg_r - mixed_r;
            let err_g = target_fg_g - mixed_g;
            let err_b = target_fg_b - mixed_b;

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
            for (dx, dy, bit) in coords {
                if x + dx < width && y + dy < height {
                    for (dx_e, dy_e, factor) in diffusion_coords {
                        let nx = x as i32 + dx as i32 + dx_e;
                        let ny = y as i32 + dy as i32 + dy_e;
                        if nx >= 0 && nx < width as i32 && ny >= 0 && ny < height as i32 {
                            error_diffusion[ny as usize][nx as usize].0 += err_r * factor;
                            error_diffusion[ny as usize][nx as usize].1 += err_g * factor;
                            error_diffusion[ny as usize][nx as usize].2 += err_b * factor;
                        }
                    }
                }
            }

            // Convert back to u8 for terminal codes
            let fr = (linear_to_srgb(final_fg_r) * 255.0).round() as u8;
            let fg = (linear_to_srgb(final_fg_g) * 255.0).round() as u8;
            let fb = (linear_to_srgb(final_fg_b) * 255.0).round() as u8;

            let br = (linear_to_srgb(bg_r) * 255.0).round() as u8;
            let bg = (linear_to_srgb(bg_g) * 255.0).round() as u8;
            let bb = (linear_to_srgb(bg_b) * 255.0).round() as u8;

            use std::fmt::Write as _;
            write!(line, "\x1b[38;2;{};{};{};48;2;{};{};{}m{}", 
                fr, fg, fb,
                br, bg, bb,
                braille_char
            ).unwrap();
        }
        println!("{}\x1b[0m", line);
    }
    Ok(())
}

/// Returns the average linear RGB color for a group of pixels
fn average_color_linear(pixels: &[PixelData]) -> Option<(f32, f32, f32)> {
    if pixels.is_empty() {
        return None;
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

    Some((r_sum / count, g_sum / count, b_sum / count))
}

/// Solves: 2 * Target - BG = Dot_Color
/// Clamps result between 0.0 and 1.0
fn solve_dot_color(target: (f32, f32, f32), bg: (f32, f32, f32)) -> (f32, f32, f32) {
    let r = (2.0 * target.0 - bg.0).clamp(0.0, 1.0);
    let g = (2.0 * target.1 - bg.1).clamp(0.0, 1.0);
    let b = (2.0 * target.2 - bg.2).clamp(0.0, 1.0);
    (r, g, b)
}

fn srgb_to_linear(c: f32) -> f32 {
    if c <= 0.04045 { c / 12.92 } else { ((c + 0.055) / 1.055).powf(2.4) }
}

fn linear_to_srgb(c: f32) -> f32 {
    if c <= 0.0031308 { 12.92 * c } else { 1.055 * c.powf(1.0 / 2.4) - 0.055 }
}