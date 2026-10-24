fn main() {
    let dir = std::path::Path::new("/mnt/f/gal/【PC硬盘】【官中】eden");
    let scr = eden_paz::PazArchive::open(dir.join("scr.paz")).unwrap();
    let bytes = scr.read_entry("B006.sc").unwrap();
    let text = eden_script::decode_script(&bytes, eden_script::ScriptEncoding::Gbk).unwrap();
    let lines: Vec<&str> = text.lines().collect();
    let needle = std::env::args().nth(1).unwrap_or_default();
    if needle.is_empty() {
        for (i, line) in lines.iter().enumerate() {
            println!("{:5} {}", i + 1, line);
        }
    } else {
        let needle = needle.to_ascii_lowercase();
        for (i, line) in lines.iter().enumerate() {
            if line.to_ascii_lowercase().contains(&needle) {
                let lo = i.saturating_sub(8);
                let hi = (i + 12).min(lines.len());
                println!("--- around line {} ---", i + 1);
                for (j, l) in lines[lo..hi].iter().enumerate() {
                    println!("{:5} {}", lo + j + 1, l);
                }
            }
        }
    }
}
