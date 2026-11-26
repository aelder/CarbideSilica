//! Film grain over the display (`FilmGrain`): a fixed pattern of fine noise that makes the
//! black a little less flat. It does not move, so it costs nothing after the first frame.
//!
//! The Swift app adds the grain with the blend mode "plus lighter". A GTK widget cannot
//! blend with the widgets below it. The tile is white with the noise in its alpha, so that
//! each point adds a little light, as "plus lighter" does on the black of the display.

use gtk::prelude::*;
use gtk::{gdk, glib, graphene};

use super::canvas::Canvas;

/// How strong the grain is. It must stay near the limit of what a person sees.
const STRENGTH: f64 = 0.05;
const SIDE: usize = 384;

fn tile() -> gdk::Texture {
    let mut bytes = vec![0u8; SIDE * SIDE * 4];
    let mut rng = rand::rng();
    for pixel in bytes.chunks_exact_mut(4) {
        // The sum of three random values is near a normal distribution.
        use rand::Rng;
        let sum: u32 = (0..3).map(|_| rng.random::<u8>() as u32).sum();
        let value = (sum / 3) as u8;
        pixel[0] = 255;
        pixel[1] = 255;
        pixel[2] = 255;
        // The strength is in the tile, so that the picture needs no opacity node.
        pixel[3] = (value as f64 * STRENGTH).round() as u8;
    }
    gdk::MemoryTexture::new(
        SIDE as i32,
        SIDE as i32,
        gdk::MemoryFormat::R8g8b8a8,
        &glib::Bytes::from_owned(bytes),
        SIDE * 4,
    )
    .upcast()
}

/// The grain, as a layer over the whole window. It takes no clicks.
pub fn film_grain() -> Canvas {
    let texture = tile();
    let canvas = Canvas::new(0, 0, move |_, snapshot, w, h| {
        // A tile of 384 pixels at a scale of 2, as the Swift app.
        let half = SIDE as f32 / 2.0;
        let mut y = 0.0;
        while y < h {
            let mut x = 0.0;
            while x < w {
                snapshot.append_texture(&texture, &graphene::Rect::new(x, y, half, half));
                x += half;
            }
            y += half;
        }
    });
    canvas.set_can_target(false);
    canvas.set_hexpand(true);
    canvas.set_vexpand(true);
    canvas
}
