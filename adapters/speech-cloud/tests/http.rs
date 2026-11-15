//! The adapters against a tiny local HTTP server that checks the request and answers like the real API.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::mpsc;

use capri_application::ports::{Audio, SpeechToText, TextToSpeech};
use capri_speech_cloud::{ElevenLabsStt, ElevenLabsTts, OpenAiStt, OpenAiTts, wav};

/// Serves one request; sends (request line, headers, body) back to the test.
fn serve_once(
    status: &'static str,
    content_type: &'static str,
    body: Vec<u8>,
) -> (String, mpsc::Receiver<(String, String, Vec<u8>)>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut reader = BufReader::new(stream.try_clone().unwrap());
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        let mut headers = String::new();
        let mut len = 0;
        loop {
            let mut h = String::new();
            reader.read_line(&mut h).unwrap();
            if h == "\r\n" {
                break;
            }
            if let Some(v) = h.to_lowercase().strip_prefix("content-length:") {
                len = v.trim().parse().unwrap();
            }
            headers.push_str(&h);
        }
        let mut req = vec![0; len];
        reader.read_exact(&mut req).unwrap();
        tx.send((line, headers, req)).unwrap();
        let head = format!(
            "HTTP/1.1 {status}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        stream.write_all(head.as_bytes()).unwrap();
        stream.write_all(&body).unwrap();
    });
    (url, rx)
}

fn phrase() -> Audio {
    Audio { rate: 16_000, samples: vec![0.1; 16_000] }
}

#[tokio::test]
async fn openai_transcription_sends_a_wav_and_reads_text() {
    let (url, rx) = serve_once("200 OK", "application/json", br#"{"text":"ouvre Spotify"}"#.to_vec());
    let mut stt = OpenAiStt::new(format!("{url}/v1"), "whisper-1");
    stt.api_key = Some("sk-test".into());
    stt.language = Some("fr".into());
    assert_eq!(stt.transcribe(&phrase()).await.unwrap(), "ouvre Spotify");
    let (line, headers, body) = rx.recv().unwrap();
    assert!(line.starts_with("POST /v1/audio/transcriptions"));
    assert!(headers.to_lowercase().contains("authorization: bearer sk-test"));
    let body = String::from_utf8_lossy(&body);
    assert!(body.contains("name=\"model\"") && body.contains("whisper-1") && body.contains("name=\"language\""));
    assert!(body.contains("RIFF"), "the audio goes up as WAV");
}

#[tokio::test]
async fn openai_speech_decodes_the_wav_answer() {
    let answer = wav::encode(&Audio { rate: 24_000, samples: vec![0.25; 2_400] });
    let (url, rx) = serve_once("200 OK", "audio/wav", answer);
    let tts = OpenAiTts::new(format!("{url}/v1"), "kokoro", "af_bella");
    let audio = tts.synthesize("Salut choom.").await.unwrap();
    assert_eq!(audio.rate, 24_000);
    assert_eq!(audio.samples.len(), 2_400);
    let (line, _, body) = rx.recv().unwrap();
    assert!(line.starts_with("POST /v1/audio/speech"));
    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["voice"], "af_bella");
    assert_eq!(json["response_format"], "wav");
}

#[tokio::test]
async fn elevenlabs_speech_reads_raw_pcm() {
    let pcm: Vec<u8> = (0..2_205).flat_map(|_| 8_192i16.to_le_bytes()).collect();
    let (url, rx) = serve_once("200 OK", "audio/pcm", pcm);
    let mut tts = ElevenLabsTts::new("xi-key", "voice123");
    tts.base_url = url;
    let audio = tts.synthesize("Salut.").await.unwrap();
    assert_eq!(audio.rate, 22_050);
    assert!((audio.samples[0] - 0.25).abs() < 0.001);
    let (line, headers, _) = rx.recv().unwrap();
    assert!(line.starts_with("POST /text-to-speech/voice123?output_format=pcm_22050"));
    assert!(headers.to_lowercase().contains("xi-api-key: xi-key"));
}

#[tokio::test]
async fn elevenlabs_refused_key_is_explained() {
    let (url, _rx) = serve_once("401 Unauthorized", "application/json", br#"{"detail":"invalid"}"#.to_vec());
    let mut stt = ElevenLabsStt::new("bad");
    stt.base_url = url;
    let e = stt.transcribe(&phrase()).await.unwrap_err();
    assert!(e.0.contains("refused"), "{}", e.0);
}
