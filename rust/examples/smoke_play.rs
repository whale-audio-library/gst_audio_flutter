use std::{env, thread, time::Duration};

use gst_audio_core::api::player;

fn main() -> Result<(), String> {
    let input = env::args().nth(1).ok_or_else(|| {
        "usage: cargo run --example smoke_play -- <audio-file-or-uri>".to_string()
    })?;

    player::init_app();
    let state = player::set_playlist(vec![input], 0)?;
    println!("loaded: {}", state.current_title);
    player::play()?;

    for _ in 0..80 {
        thread::sleep(Duration::from_millis(100));
        let state = player::get_state()?;
        println!(
            "playing={} position={} duration={} error={}",
            state.is_playing, state.position_ms, state.duration_ms, state.last_error
        );
        if !state.last_error.is_empty() {
            let _ = player::shutdown_player();
            return Err(state.last_error);
        }
        if !state.is_playing && state.position_ms > 0 {
            break;
        }
    }

    player::shutdown_player()?;
    Ok(())
}
