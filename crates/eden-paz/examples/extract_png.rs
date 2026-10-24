fn main() {
    let dir = std::path::Path::new("/mnt/f/gal/【PC硬盘】【官中】eden");
    let mut args = std::env::args().skip(1);
    let archive_name = args.next().unwrap();
    let entry = args.next().unwrap();
    let out = args.next().unwrap();
    let ar = eden_paz::PazArchive::open(dir.join(archive_name)).unwrap();
    let img = ar.decode_png(&entry).unwrap();
    std::fs::write(&out, eden_paz::encode_png(&img).unwrap()).unwrap();
    println!("{} {}x{} -> {}", entry, img.width, img.height, out);
}
