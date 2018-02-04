use byteorder::{BigEndian, ByteOrder, LittleEndian, ReadBytesExt, WriteBytesExt};
use constants::VOICE_GATEWAY_VERSION;
use internal::prelude::*;
use internal::ws_impl::{ReceiverExt, SenderExt};
use internal::Timer;
use model::event::VoiceEvent;
use model::id::UserId;
use opus::{
    packet as opus_packet,
    Application as CodingMode,
    Bitrate,
    Channels,
    Decoder as OpusDecoder,
    Encoder as OpusEncoder,
    SoftClip,
};
use parking_lot::Mutex;
use rand::random;
use serde::Deserialize;
use sodiumoxide::crypto::secretbox::{self, Key, Nonce};
use std::collections::HashMap;
use std::io::Write;
use std::net::{SocketAddr, ToSocketAddrs, UdpSocket};
use std::sync::mpsc::{self, Receiver as MpscReceiver, Sender as MpscSender};
use std::sync::Arc;
use std::thread::{self, Builder as ThreadBuilder, JoinHandle};
use std::time::Duration;
use super::audio::{AudioReceiver, AudioType, HEADER_LEN, SAMPLE_RATE, DEFAULT_BITRATE, LockedAudio};
use super::connection_info::ConnectionInfo;
use super::{payload, VoiceError, CRYPTO_MODE};
use websocket::client::Url as WebsocketUrl;
use websocket::sync::client::ClientBuilder;
use websocket::sync::stream::{AsTcpStream, TcpStream, TlsStream};
use websocket::sync::Client as WsClient;

type Client = WsClient<TlsStream<TcpStream>>;

enum ReceiverStatus {
    Udp(Vec<u8>),
    Websocket(VoiceEvent),
}

#[allow(dead_code)]
struct ThreadItems {
    rx: MpscReceiver<ReceiverStatus>,
    udp_close_sender: MpscSender<i32>,
    udp_thread: JoinHandle<()>,
    ws_close_sender: MpscSender<i32>,
    ws_thread: JoinHandle<()>,
}

#[allow(dead_code)]
pub struct Connection {
    audio_timer: Timer,
    client: Arc<Mutex<Client>>,
    decoder_map: HashMap<(u32, Channels), OpusDecoder>,
    destination: SocketAddr,
    encoder: OpusEncoder,
    encoder_stereo: bool,
    keepalive_timer: Timer,
    key: Key,
    last_heartbeat_nonce: Option<u64>,
    soft_clip: SoftClip,
    sequence: u16,
    silence_frames: u8,
    speaking: bool,
    ssrc: u32,
    thread_items: ThreadItems,
    timestamp: u32,
    udp: UdpSocket,
    user_id: UserId,
}

impl Connection {
    pub fn new(mut info: ConnectionInfo) -> Result<Connection> {
        let url = generate_url(&mut info.endpoint)?;

        let mut client = ClientBuilder::from_url(&url).connect_secure(None)?;
        let mut hello = None;
        let mut ready = None;
        client.send_json(&payload::build_identify(&info))?;

        loop {
            if hello.is_some() && ready.is_some() {
                break;
            }

            let value = match client.recv_json()? {
                Some(value) => value,
                None => continue,
            };

            match VoiceEvent::deserialize(value)? {
                VoiceEvent::Ready(r) => {
                    ready = Some(r);
                },
                VoiceEvent::Hello(h) => {
                    hello = Some(h);
                },
                other => {
                    debug!("[Voice] Expected ready/hello; got: {:?}", other);

                    return Err(Error::Voice(VoiceError::ExpectedHandshake));
                },
            }
        };

        let hello = hello.unwrap();
        let ready = ready.unwrap();

        if !has_valid_mode(&ready.modes) {
            return Err(Error::Voice(VoiceError::VoiceModeUnavailable));
        }

        let destination = (&info.endpoint[..], ready.port)
            .to_socket_addrs()?
            .next()
            .ok_or(Error::Voice(VoiceError::HostnameResolve))?;

        // Important to note here: the length of the packet can be of either 4
        // or 70 bytes. If it is 4 bytes, then we need to send a 70-byte packet
        // to determine the IP.
        //
        // Past the initial 4 bytes, the packet _must_ be completely empty data.
        //
        // The returned packet will be a null-terminated string of the IP, and
        // the port encoded in LE in the last two bytes of the packet.
        let udp = UdpSocket::bind("0.0.0.0:0")?;

        {
            let mut bytes = [0; 70];

            (&mut bytes[..]).write_u32::<BigEndian>(ready.ssrc)?;
            udp.send_to(&bytes, destination)?;

            let mut bytes = [0; 256];
            let (len, _addr) = udp.recv_from(&mut bytes)?;

            // Find the position in the bytes that contains the first byte of 0,
            // indicating the "end of the address".
            let index = bytes
                .iter()
                .skip(4)
                .position(|&x| x == 0)
                .ok_or(Error::Voice(VoiceError::FindingByte))?;

            let pos = 4 + index;
            let addr = String::from_utf8_lossy(&bytes[4..pos]);
            let port_pos = len - 2;
            let port = (&bytes[port_pos..]).read_u16::<LittleEndian>()?;

            client
                .send_json(&payload::build_select_protocol(addr, port))?;
        }

        let key = encryption_key(&mut client)?;

        let _ = client
            .stream_ref()
            .as_tcp()
            .set_read_timeout(Some(Duration::from_millis(25)));

        let mutexed_client = Arc::new(Mutex::new(client));
        let thread_items = start_threads(Arc::clone(&mutexed_client), &udp)?;

        info!("[Voice] Connected to: {}", info.endpoint);

        // Encode for Discord in Stereo, as required.
        let mut encoder = OpusEncoder::new(SAMPLE_RATE, Channels::Stereo, CodingMode::Audio)?;
        encoder.set_bitrate(Bitrate::Bits(DEFAULT_BITRATE))?;
        let soft_clip = SoftClip::new(Channels::Stereo);

        // Per discord dev team's current recommendations:
        // (https://discordapp.com/developers/docs/topics/voice-connections#heartbeating)
        let temp_heartbeat = (hello.heartbeat_interval as f64 * 0.75) as u64;

        Ok(Connection {
            audio_timer: Timer::new(1000 * 60 * 4),
            client: mutexed_client,
            decoder_map: HashMap::new(),
            destination,
            encoder,
            encoder_stereo: false,
            key,
            keepalive_timer: Timer::new(temp_heartbeat),
            last_heartbeat_nonce: None,
            udp,
            sequence: 0,
            silence_frames: 0,
            soft_clip,
            speaking: false,
            ssrc: ready.ssrc,
            thread_items,
            timestamp: 0,
            user_id: info.user_id,
        })
    }

    #[allow(unused_variables)]
    pub fn cycle(&mut self,
                 sources: &mut Vec<LockedAudio>,
                 receiver: &mut Option<Box<AudioReceiver>>,
                 audio_timer: &mut Timer,
                 bitrate: Bitrate)
                 -> Result<()> {
        // We need to actually reserve enough space for the desired bitrate.
        let size = match bitrate {
            // If user specified, we can calculate. 20ms means 50fps.
            Bitrate::Bits(b) => b.abs()/50,
            // Otherwise, just have a lot preallocated.
            _ => 5120,
        } + 16;

        let mut buffer = [0i16; 960 * 2];
        let mut mix_buffer = [0f32; 960 * 2];
        let mut packet = vec![0u8; size as usize].into_boxed_slice();
        let mut nonce = secretbox::Nonce([0; 24]);

        if let Some(receiver) = receiver.as_mut() {
            while let Ok(status) = self.thread_items.rx.try_recv() {
                match status {
                    ReceiverStatus::Udp(packet) => {
                        let mut handle = &packet[2..];
                        let seq = handle.read_u16::<BigEndian>()?;
                        let timestamp = handle.read_u32::<BigEndian>()?;
                        let ssrc = handle.read_u32::<BigEndian>()?;

                        nonce.0[..HEADER_LEN]
                            .clone_from_slice(&packet[..HEADER_LEN]);

                        if let Ok(mut decrypted) =
                            secretbox::open(&packet[HEADER_LEN..], &nonce, &self.key) {
                            let channels = opus_packet::get_nb_channels(&decrypted)?;

                            let entry =
                                self.decoder_map.entry((ssrc, channels)).or_insert_with(
                                    || OpusDecoder::new(SAMPLE_RATE, channels).unwrap(),
                                );

                            // Strip RTP Header Extensions (one-byte)
                            if decrypted[0] == 0xBE && decrypted[1] == 0xDE {
                                // Read the length bytes as a big-endian u16.
                                let header_extension_len = BigEndian::read_u16(&decrypted[2..4]);
                                let mut offset = 4;
                                for _ in 0..header_extension_len {
                                    let byte = decrypted[offset];
                                    offset += 1;
                                    if byte == 0 {
                                        continue;
                                    }

                                    offset += 1 + (0b1111 & (byte >> 4)) as usize;
                                }

                                while decrypted[offset] == 0 {
                                    offset += 1;
                                }

                                decrypted = decrypted.split_off(offset);
                            }

                            let len = entry.decode(&decrypted, &mut buffer, false)?;

                            let is_stereo = channels == Channels::Stereo;

                            let b = if is_stereo { len * 2 } else { len };

                            receiver
                                .voice_packet(ssrc, seq, timestamp, is_stereo, &buffer[..b]);
                        }
                    },
                    ReceiverStatus::Websocket(VoiceEvent::Speaking(ev)) => {
                        receiver.speaking_update(ev.ssrc, ev.user_id.0, ev.speaking);
                    },
                    ReceiverStatus::Websocket(VoiceEvent::HeartbeatAck(ev)) => {
                        match self.last_heartbeat_nonce {
                            Some(nonce) => {
                                if ev.nonce != nonce {
                                    warn!("[Voice] Heartbeat nonce mismatch! Expected {}, saw {}.", nonce, ev.nonce);
                                }
                                self.last_heartbeat_nonce = None;
                            },
                            None => {},
                        }
                    },
                    ReceiverStatus::Websocket(other) => {
                        info!("[Voice] Received other websocket data: {:?}", other);
                    },
                }
            }
        } else {
            loop {
                if self.thread_items.rx.try_recv().is_err() {
                    break;
                }
            }
        }

        // Send the voice websocket keepalive if it's time
        if self.keepalive_timer.check() {
            let nonce = random::<u64>();
            self.last_heartbeat_nonce = Some(nonce);
            self.client.lock().send_json(&payload::build_heartbeat(nonce))?;
        }

        // Send UDP keepalive if it's time
        if self.audio_timer.check() {
            let mut bytes = [0; 4];
            (&mut bytes[..]).write_u32::<BigEndian>(self.ssrc)?;
            self.udp.send_to(&bytes, self.destination)?;
        }

        // Reconfigure encoder bitrate.
        // From my testing, it seemed like this needed to be set every cycle.
        // -- FelixMCFelix
        match self.encoder.set_bitrate(bitrate) {
            Ok(_) => {},
            Err(e) => {println!("Bitrate set unsuccessfully: {:?}", e);},
        }

        let mut opus_frame = Vec::new();

        let mut len = 0;

        // Walk over all the audio files, removing those which have finished.
        // For this purpose, we need a while loop in Rust.
        let mut i = 0;

        while i < sources.len() {
            let mut finished = false;

            let aud_lock = (&sources[i]).clone();
            let mut aud = aud_lock.lock();

            let vol = aud.volume;
            let skip = !aud.playing;

            {
                let stream = &mut aud.source;

                if skip {
                    i += 1;

                    continue;
                }

                // Assume this for now, at least.
                // We'll be fusing streams, so we can either keep
                // as stereo or downmix to mono.
                let is_stereo = true;
                let source_stereo = stream.is_stereo();

                if is_stereo != self.encoder_stereo {
                    let channels = if is_stereo {
                        Channels::Stereo
                    } else {
                        Channels::Mono
                    };
                    self.encoder = OpusEncoder::new(SAMPLE_RATE, channels, CodingMode::Audio)?;
                    self.encoder_stereo = is_stereo;
                }

                let temp_len = match stream.get_type() {
                    AudioType::Opus => match stream.decode_and_add_opus_frame(&mut mix_buffer, vol) {
                        Some(frame) => {
                            opus_frame.len()
                        },
                        None => 0,
                    },
                    AudioType::Pcm => {
                        let buffer_len = if source_stereo { 960 * 2 } else { 960 };

                        match stream.read_pcm_frame(&mut buffer[..buffer_len]) {
                            Some(len) => len,
                            None => 0,
                        }
                    },
                };

                // May need to force interleave/copy.
                combine_audio(buffer, &mut mix_buffer, source_stereo, vol);

                len = len.max(temp_len);
                i += if temp_len > 0 {
                    1
                } else {
                    sources.remove(i);
                    finished = true;

                    0
                };
            }

            aud.finished = finished;
            if !finished {
                aud.step_frame();
            }
        };

        self.soft_clip.apply(&mut mix_buffer);

        if len == 0 {
            if self.silence_frames > 0 {
                self.silence_frames -= 1;

                // Explicit "Silence" frame.
                opus_frame.extend_from_slice(&[0xf8, 0xff, 0xfe]);
            } else {
                // Per official guidelines, send 5x silence BEFORE we stop speaking.
                self.set_speaking(false)?;

                audio_timer.await();

                return Ok(());
            }
        } else {
            self.silence_frames = 5;

            for value in &mut buffer[len..] {
                *value = 0;
            }
        }

        self.set_speaking(true)?;

        let index = self.prep_packet(&mut packet, mix_buffer, &opus_frame, nonce)?;
        audio_timer.await();

        self.udp.send_to(&packet[..index], self.destination)?;
        self.audio_timer.reset();

        Ok(())
    }

    fn prep_packet(&mut self,
                   packet: &mut [u8],
                   buffer: [f32; 1920],
                   opus_frame: &[u8],
                   mut nonce: Nonce)
                   -> Result<usize> {
        {
            let mut cursor = &mut packet[..HEADER_LEN];
            cursor.write_all(&[0x80, 0x78])?;
            cursor.write_u16::<BigEndian>(self.sequence)?;
            cursor.write_u32::<BigEndian>(self.timestamp)?;
            cursor.write_u32::<BigEndian>(self.ssrc)?;
        }

        nonce.0[..HEADER_LEN]
            .clone_from_slice(&packet[..HEADER_LEN]);

        let sl_index = packet.len() - 16;
        let buffer_len = if self.encoder_stereo { 960 * 2 } else { 960 };

        let len = if opus_frame.is_empty() {
            self.encoder
                .encode_float(&buffer[..buffer_len], &mut packet[HEADER_LEN..sl_index])?
        } else {
            let len = opus_frame.len();
            packet[HEADER_LEN..HEADER_LEN + len]
                .clone_from_slice(opus_frame);
            len
        };

        let crypted = {
            let slice = &packet[HEADER_LEN..HEADER_LEN + len];
            secretbox::seal(slice, &nonce, &self.key)
        };
        let index = HEADER_LEN + crypted.len();
        packet[HEADER_LEN..index].clone_from_slice(&crypted);

        self.sequence = self.sequence.wrapping_add(1);
        self.timestamp = self.timestamp.wrapping_add(960);

        Ok(HEADER_LEN + crypted.len())
    }

    fn set_speaking(&mut self, speaking: bool) -> Result<()> {
        if self.speaking == speaking {
            return Ok(());
        }

        self.speaking = speaking;

        self.client.lock().send_json(&payload::build_speaking(speaking))
    }
}

impl Drop for Connection {
    fn drop(&mut self) {
        let _ = self.thread_items.udp_close_sender.send(0);
        let _ = self.thread_items.ws_close_sender.send(0);

        info!("[Voice] Disconnected");
    }
}

#[inline]
fn combine_audio(
    raw_buffer: [i16; 1920],
    float_buffer: &mut [f32; 1920],
    true_stereo: bool,
    volume: f32,
) {
    for i in 0..1920 {
        let sample_index = if true_stereo { i } else { i/2 };
        let sample = (raw_buffer[sample_index] as f32) / 32768.0;

        float_buffer[i] = float_buffer[i] + sample*volume;
    }
}

fn generate_url(endpoint: &mut String) -> Result<WebsocketUrl> {
    if endpoint.ends_with(":80") {
        let len = endpoint.len();

        endpoint.truncate(len - 3);
    }

    WebsocketUrl::parse(&format!("wss://{}/?v={}", endpoint, VOICE_GATEWAY_VERSION))
        .or(Err(Error::Voice(VoiceError::EndpointUrl)))
}

#[inline]
fn encryption_key(client: &mut Client) -> Result<Key> {
    loop {
        let value = match client.recv_json()? {
            Some(value) => value,
            None => continue,
        };

        match VoiceEvent::deserialize(value)? {
            VoiceEvent::SessionDescription(desc) => {
                if desc.mode != CRYPTO_MODE {
                    return Err(Error::Voice(VoiceError::VoiceModeInvalid));
                }

                return Key::from_slice(&desc.secret_key)
                    .ok_or(Error::Voice(VoiceError::KeyGen));
            },
            VoiceEvent::Unknown(op, value) => {
                debug!(
                    "[Voice] Expected ready for key; got: op{}/v{:?}",
                    op.num(),
                    value
                );
            },
            _ => {},
        }
    }
}

#[inline]
fn has_valid_mode<T, It> (modes: It) -> bool
where T: for<'a> PartialEq<&'a str>,
      It : IntoIterator<Item=T>
{
    modes.into_iter().any(|s| s == CRYPTO_MODE)
}

#[inline]
fn start_threads(client: Arc<Mutex<Client>>, udp: &UdpSocket) -> Result<ThreadItems> {
    let (udp_close_sender, udp_close_reader) = mpsc::channel();
    let (ws_close_sender, ws_close_reader) = mpsc::channel();

    let current_thread = thread::current();
    let thread_name = current_thread.name().unwrap_or("serenity voice");

    let (tx, rx) = mpsc::channel();
    let tx_clone = tx.clone();
    let udp_clone = udp.try_clone()?;

    let udp_thread = ThreadBuilder::new()
        .name(format!("{} UDP", thread_name))
        .spawn(move || {
            let _ = udp_clone.set_read_timeout(Some(Duration::from_millis(250)));

            let mut buffer = [0; 512];

            loop {
                if let Ok((len, _)) = udp_clone.recv_from(&mut buffer) {
                    let piece = buffer[..len].to_vec();
                    let send = tx.send(ReceiverStatus::Udp(piece));

                    if send.is_err() {
                        return;
                    }
                } else if udp_close_reader.try_recv().is_ok() {
                    return;
                }
            }
        })?;

    let ws_thread = ThreadBuilder::new()
        .name(format!("{} WS", thread_name))
        .spawn(move || loop {
            while let Ok(Some(value)) = client.lock().recv_json() {
                let msg = match VoiceEvent::deserialize(value) {
                    Ok(msg) => msg,
                    Err(why) => {
                        warn!("Error deserializing voice event: {:?}", why);

                        break;
                    },
                };

                if tx_clone.send(ReceiverStatus::Websocket(msg)).is_err() {
                    return;
                }
            }

            if ws_close_reader.try_recv().is_ok() {
                return;
            }

            thread::sleep(Duration::from_millis(25));
        })?;

    Ok(ThreadItems {
        rx: rx,
        udp_close_sender: udp_close_sender,
        udp_thread: udp_thread,
        ws_close_sender: ws_close_sender,
        ws_thread: ws_thread,
    })
}
