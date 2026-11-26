// reads the sound out of a video or audio file into a test recording, to check a real file works
// cargo run --release --example import_preview -- <video or audio file>

fn main() {
    let source = std::path::PathBuf::from(std::env::args().nth(1).expect("which file?"));
    let out = std::env::temp_dir().join("lecture-import-preview.wav");
    let started = std::time::Instant::now();
    match lecture_lib::importer::sound_to_wav(&source, &out) {
        Ok(length_ms) => {
            let seconds = length_ms / 1000;
            println!("ok: {}:{:02} of sound, read in {:.1}s", seconds / 60, seconds % 60, started.elapsed().as_secs_f32());
            println!("listen to it: open {}", out.display());
        }
        Err(error) => println!("error: {error}"),
    }
}
