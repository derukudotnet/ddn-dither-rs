// Verification harness (std + ddn_dither only). Reads/writes binary PNM so Python can compare.
//   harness resize IN.pgm DW DH X0 Y0 X1 Y1 OUT.pgm
//   harness render IN.ppm OUT.pgm out_w=800 out_h=350 ps=0 frame=none|builtin scale=4 ac=1 fit=cover|contain|pad box=x0,y0,x1,y1
use ddn_dither::autocontrast;
use ddn_dither::frame::Frame;
use ddn_dither::gray::{Plane, RgbImage};
use ddn_dither::pipeline::{render, Fit, Params};
use ddn_dither::resample::{resize_plane, BoxF};
use std::collections::HashMap;
use std::fs;

fn read_pnm(path: &str) -> (String, usize, usize, Vec<u8>) {
    let b = fs::read(path).unwrap();
    let mut pos = 0;
    let mut tok = Vec::new();
    while tok.len() < 4 {
        while b[pos].is_ascii_whitespace() { pos += 1; }
        let s = pos;
        while !b[pos].is_ascii_whitespace() { pos += 1; }
        tok.push(String::from_utf8(b[s..pos].to_vec()).unwrap());
    }
    pos += 1;
    (tok[0].clone(), tok[1].parse().unwrap(), tok[2].parse().unwrap(), b[pos..].to_vec())
}

fn write_pgm(path: &str, w: usize, h: usize, d: &[u8]) {
    let mut out = format!("P5\n{w} {h}\n255\n").into_bytes();
    out.extend_from_slice(d);
    fs::write(path, out).unwrap();
}

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    match a[0].as_str() {
        "resize" => {
            let (_, w, h, d) = read_pnm(&a[1]);
            let f = |i: usize| a[i].parse::<f64>().unwrap();
            let p = Plane::new(w, h, d);
            let o = resize_plane(&p, a[2].parse().unwrap(), a[3].parse().unwrap(), BoxF { x0: f(4), y0: f(5), x1: f(6), y1: f(7) });
            write_pgm(&a[8], o.w, o.h, &o.data);
        }
        "autocontrast" => {
            let (_, w, h, d) = read_pnm(&a[1]);
            let mut p = Plane::new(w, h, d);
            autocontrast::autocontrast(&mut p, autocontrast::DEFAULT_CUTOFF);
            write_pgm(&a[2], w, h, &p.data);
        }
        "render" => {
            let (_, w, h, d) = read_pnm(&a[1]);
            let kv: HashMap<String, String> = a[3..].iter().filter_map(|s| s.split_once('=')).map(|(k, v)| (k.to_string(), v.to_string())).collect();
            let g = |k: &str, d: &str| kv.get(k).cloned().unwrap_or(d.to_string());
            let gray = RgbImage::new(w, h, d).to_gray();
            let frame = if g("frame", "none") == "builtin" { Some(Frame::builtin()) } else { None };
            let bx: Vec<f64> = g("box", &format!("0,0,{w},{h}")).split(',').map(|s| s.parse().unwrap()).collect();
            let fit = match g("fit", "cover").as_str() {
                "contain" => Fit::Contain,
                "pad" => Fit::EdgeRepeat,
                _ => Fit::Cover(BoxF { x0: bx[0], y0: bx[1], x1: bx[2], y1: bx[3] }),
            };
            let p = Params {
                fit,
                out_w: g("out_w", "800").parse().unwrap(),
                out_h: g("out_h", "350").parse().unwrap(),
                pixel_size: g("ps", "0").parse().unwrap(),
                frame: frame.as_ref(),
                frame_scale: g("scale", "4").parse().unwrap(),
                autocontrast: g("ac", "1") == "1",
                contrast: g("contrast", "1").parse().unwrap(),
            };
            let r = render(&gray, &p);
            write_pgm(&a[2], r.grid_w, r.grid_h, &r.levels);
            println!("{} {} {} {} {}", r.grid_w, r.grid_h, r.scale, r.out_w, r.out_h);
        }
        _ => panic!("unknown mode"),
    }
}
