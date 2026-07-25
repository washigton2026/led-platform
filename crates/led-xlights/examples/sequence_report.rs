fn main() {
    let seq = led_xlights::parse_sequence_file(std::path::Path::new(
        "/Users/gabrielabambam/Desktop/meu show robô/__.xsq")).unwrap();
    println!("media: {:?}", seq.media);
    println!("duration: {}ms", seq.duration_ms);
    println!("spans: {}", seq.spans.len());
    for s in &seq.spans {
        println!("  {:<10} {:>7}..{:<7} @ {}", s.effect, s.start_ms, s.end_ms, s.element);
    }
}
