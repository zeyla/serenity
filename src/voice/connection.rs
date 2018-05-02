use byteorder::{BigEndian, ByteOrder, LittleEndian, ReadBytesExt, WriteBytesExt};
use constants::VOICE_GATEWAY_VERSION;
use future_utils::{
    mpsc::{unbounded, UnboundedReceiver, UnboundedSender},
    StreamExt,
};
use futures::{
    future::{
        loop_fn,
        ok,
        result,
        Future,
        Loop,
    },
    stream::{
        SplitSink,
        SplitStream,
    },
    sync::{
        mpsc::{
            self as future_mpsc,
            Receiver as FutureMpscReceiver,
            Sender as FutureMpscSender,
        },
        oneshot::{
            channel as oneshot_channel,
            Sender as OneShotSender,
        },
    },
    Sink,
    Stream,
};
use internal::prelude::*;
use internal::ws_ext::{
    message_to_json,
    ReceiverExt,
    SenderExt,
    WsClient,
};
use internal::Timer;
use log::LogLevel;
use model::event::{
    VoiceEvent,
    VoiceHello,
    VoiceReady,
};
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
use std::{
    collections::HashMap,
    io::{Error as IoError, Write},
    net::{SocketAddr, ToSocketAddrs},
    sync::mpsc::{self, Receiver as MpscReceiver, Sender as MpscSender},
    sync::Arc,
    thread::{self, Builder as ThreadBuilder, JoinHandle},
    time::Duration,
};
use super::audio::{AudioReceiver, AudioType, HEADER_LEN, SAMPLE_RATE, DEFAULT_BITRATE, LockedAudio};
use super::connection_info::ConnectionInfo;
use super::{payload, VoiceError, CRYPTO_MODE};
use tokio_core::{
    net::{
        TcpStream,
        UdpCodec,
        UdpFramed,
        UdpSocket,
    },
    reactor::{Core, Handle, Remote},
};
use tokio_tungstenite::{connect_async, WebSocketStream};
use tungstenite::Message;
use url::Url;
// use websocket::client::Url as WebsocketUrl;
// use websocket::sync::client::ClientBuilder;
// use websocket::sync::stream::{AsTcpStream, TcpStream, TlsStream};
// use websocket::sync::Client as WsClient;

// type Client = WsClient<TlsStream<TcpStream>>;

enum ReceiverStatus {
    Udp(Vec<u8>),
    Websocket(VoiceEvent),
}

struct ProgressingVoiceHandshake {
    hello: Option<VoiceHello>,
    ready: Option<VoiceReady>,
    ws: WsClient,
}

impl ProgressingVoiceHandshake {
    fn finalize(self) -> Result<VoiceHandshake> {
        let ready = self.ready.ok_or(Error::Voice(VoiceError::ExpectedHandshake))?;
        let hello = self.hello.ok_or(Error::Voice(VoiceError::ExpectedHandshake))?;

        Ok(VoiceHandshake {
            ready,
            hello,
            ws: self.ws,
        })
    }
}

struct VoiceHandshake {
    hello: VoiceHello,
    ready: VoiceReady,
    ws: WsClient,
}

struct ListenerItems {
    close_sender: OneShotSender<()>,
    rx: UnboundedReceiver<ReceiverStatus>,
    rx_pong: UnboundedReceiver<Vec<u8>>,
}

struct VoiceCodec {
    destination: SocketAddr,
    key: Key,
    sequence: u16,
    ssrc: u32,
    timestamp: u32,
}

impl UdpCodec for VoiceCodec {
    type In = Vec<u8>;
    type Out = Vec<u8>;

    // TODO: Implement!
    fn decode(&mut self, src: &SocketAddr, buf: &[u8]) -> StdResult<Self::In, IoError> {
        Ok(vec![0u8])
    }

    // TODO: Implement.
    // User will either send a heartbeat or audio of variable length.
    // Make an Enum for this I guess?
    fn encode(&mut self, msg: Self::Out, buf: &mut Vec<u8>) -> SocketAddr {
        self.destination
    }
}

#[allow(dead_code)]
pub struct Connection {
    audio_timer: Timer,
    decoder_map: HashMap<(u32, Channels), OpusDecoder>,
    destination: SocketAddr,
    encoder: OpusEncoder,
    encoder_stereo: bool,
    keepalive_timer: Timer,
    last_heartbeat_nonce: Option<u64>,
    listener_items: ListenerItems,
    udp_send: SplitSink<UdpFramed<VoiceCodec>>,
    silence_frames: u8,
    soft_clip: SoftClip,
    speaking: bool,
    user_id: UserId,
    ws_send: SplitSink<WsClient>,
}

impl Connection {
    pub fn new(mut info: ConnectionInfo)//, handle: Handle)
            -> Box<Future<Item = Connection, Error = Error>> {

        let mut core = Core::new().unwrap();
        let handle = core.handle();

        let url = generate_url(&mut info.endpoint);
        let local_remote_ws = handle.remote().clone();
        let local_remote_listeners = handle.remote().clone();

        let done = result(url)
            // Build a (TLS'd) websocket.
            .and_then(move |url| connect_async(url, local_remote_ws).map_err(Error::from))
            // Our init of the handshake.
            .and_then(|(ws, _)| ws.send_json(&payload::build_identify(&info)))
            // The reply has TWO PARTS, which can come in any order.
            .and_then(|ws| {
                loop_fn(ProgressingVoiceHandshake {ready: None, hello: None, ws}, |state| {
                    state.ws.recv_json()
                        .map_err(|(err, _)| err)
                        .and_then(move |(value_wrap, ws)| {
                            state.ws = ws;

                            let value = match value_wrap {
                                Some(json_value) => json_value,
                                None => {return Ok(Loop::Continue(state));},
                            };

                            match VoiceEvent::deserialize(value)? {
                                VoiceEvent::Ready(r) => {
                                    state.ready = Some(r);
                                    if state.hello.is_some(){
                                        return Ok(Loop::Break(state));
                                    }
                                },
                                VoiceEvent::Hello(h) => {
                                    state.hello = Some(h);
                                    if state.ready.is_some() {
                                        return Ok(Loop::Break(state));
                                    }
                                },
                                other => {
                                    debug!("[Voice] Expected ready/hello; got: {:?}", other);

                                    return Err(Error::Voice(VoiceError::ExpectedHandshake));
                                },
                            }

                            Ok(Loop::Continue(state))
                        })
                })
            })
            .and_then(|state| {
                let handshake = state.finalize()?;

                if !has_valid_mode(&handshake.ready.modes) {
                    return Err(Error::Voice(VoiceError::VoiceModeUnavailable));
                }

                let destination = (&info.endpoint[..], handshake.ready.port)
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

                // TODO: compute local socket addr as a lazy static
                let local = "0.0.0.0:0"
                    .to_socket_addrs()?
                    .next()
                    .ok_or(Error::Voice(VoiceError::HostnameResolve))?;

                let udp = UdpSocket::bind(&local, &handle)?;

                let mut bytes = [0u8; 70];
                let mut write_bytes = [0u8; 256];
                (&mut bytes[..]).write_u32::<BigEndian>(handshake.ready.ssrc)?;

                Ok(udp.send_dgram(&bytes[..], destination)
                    .and_then(|(udp, _)| udp.recv_dgram(vec![0u8; 256]))
                    .map_err(Error::from)
                    .and_then(move |(udp, data, len, _)| {
                        // Find the position in the bytes that contains the first byte of 0,
                        // indicating the "end of the address".
                        let index = data.iter()
                            .skip(4)
                            .position(|&x| x == 0)
                            .ok_or(Error::Voice(VoiceError::FindingByte))?;

                        let pos = 4 + index;
                        let addr = String::from_utf8_lossy(&bytes[4..pos]);
                        let port_pos = len - 2;
                        let port = (&bytes[port_pos..]).read_u16::<LittleEndian>()?;

                        Ok(handshake.ws.send_json(&payload::build_select_protocol(addr, port))
                            .map_err(Error::from)
                            .map(move |ws| {
                                handshake.ws = ws;
                                (handshake, udp, destination)
                            }))
                    })
                )
            })
            .flatten()
            .flatten()
            .and_then(|(handshake, udp, destination)| {
                Ok(encryption_key(handshake.ws)
                    .map(move |(key, ws)| {
                        handshake.ws = ws;
                        (handshake, udp, key, destination)
                    })
                )
            })
            .flatten()
            .and_then(|(handshake, udp, key, destination)| {
                let VoiceHandshake { hello, ready, ws } = handshake;
                let codec = VoiceCodec {
                    destination,
                    key,
                    sequence: 0,
                    ssrc: ready.ssrc,
                    timestamp: 0,
                };

                let (ws_send, ws_reader) = ws.split();
                let (udp_send, udp_reader) = udp.framed(codec).split();

                let listener_items = spawn_receive_handlers(ws_reader, udp_reader, &local_remote_listeners);

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
                    decoder_map: HashMap::new(),
                    destination,
                    encoder,
                    encoder_stereo: false,
                    keepalive_timer: Timer::new(temp_heartbeat),
                    last_heartbeat_nonce: None,
                    listener_items,
                    udp_send,
                    silence_frames: 100,
                    soft_clip,
                    speaking: false,
                    user_id: info.user_id,
                    ws_send,
                })
            });

        return Box::new(done);
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
            Bitrate::Bits(b) => b.abs() / 50,
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
                    // TODO: move to codec.
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

        // TODO: move to codec
        // Send UDP keepalive if it's time
        if self.audio_timer.check() {
            let mut bytes = [0; 4];
            (&mut bytes[..]).write_u32::<BigEndian>(self.ssrc)?;
            self.udp.send_to(&bytes, self.destination)?;
        }

        // Reconfigure encoder bitrate.
        // From my testing, it seemed like this needed to be set every cycle.
        if let Err(e) = self.encoder.set_bitrate(bitrate) {
            warn!("[Voice] Bitrate set unsuccessfully: {:?}", e);
        }

        let mut opus_frame = Vec::new();

        let mut len = 0;

        // TODO: Could we parallelise this across futures?
        // It's multiple I/O operations, potentially.

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

    // TODO: move to codec.
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

        float_buffer[i] = float_buffer[i] + sample * volume;
    }
}

fn generate_url(endpoint: &mut String) -> Result<Url> {
    if endpoint.ends_with(":80") {
        let len = endpoint.len();

        endpoint.truncate(len - 3);
    }

    Url::parse(&format!("wss://{}/?v={}", endpoint, VOICE_GATEWAY_VERSION))
        .or(Err(Error::Voice(VoiceError::EndpointUrl)))
}

#[inline]
fn encryption_key(ws: WsClient) -> Box<Future<Item=(Key, WsClient), Error=Error>> {
    let out = loop_fn(ws, |ws| {
        ws.recv_json()
            .map_err(|(err, _)| err)
            .and_then(|(value_wrap, ws)| {
                let value = match value_wrap {
                    Some(json_value) => json_value,
                    None => {return Ok(Loop::Continue(ws));},
                };

                match VoiceEvent::deserialize(value)? {
                    VoiceEvent::SessionDescription(desc) => {
                        if desc.mode != CRYPTO_MODE {
                            return Err(Error::Voice(VoiceError::VoiceModeInvalid));
                        }

                        let key = Key::from_slice(&desc.secret_key)
                            .ok_or(Error::Voice(VoiceError::KeyGen))?;

                        return Ok(Loop::Break((key, ws)))
                    },
                    VoiceEvent::Unknown(op, value) => {
                        debug!(
                            "[Voice] Expected ready for key; got: op{}/v{:?}",
                            op.num(),
                            value
                        );
                    },
                }

                Ok(Loop::Continue(ws))
            })
    });

    Box::new(out)
}

#[inline]
fn has_valid_mode<T, It> (modes: It) -> bool
where T: for<'a> PartialEq<&'a str>,
      It : IntoIterator<Item=T>
{
    modes.into_iter().any(|s| s == CRYPTO_MODE)
}

#[inline]
fn spawn_receive_handlers(ws: SplitStream<WsClient>, udp: SplitStream<UdpFramed<VoiceCodec>>, context: &Remote) -> ListenerItems {
    let (close_sender, close_reader) = oneshot_channel::<()>();

    let close_reader = close_reader;
    let close_reader1 = close_reader.shared();
    let close_reader2 = close_reader1.clone();

    let (tx, rx) = unbounded();
    let tx_clone = tx.clone();

    let (tx_pong, rx_pong) = unbounded();

    context.spawn(move |_|
        ws.map_err(Error::from)
            .until(close_reader1.map(|v| *v))
            .for_each(|message| {
                message_to_json(message, tx_pong).and_then(
                    |maybe_value| match maybe_value {
                        Some(value) => match VoiceEvent::deserialize(value) {
                            Ok(msg) => tx.unbounded_send(ReceiverStatus::Websocket(msg))
                                .map_err(|e| Error::FutureMpsc("WS event receiver hung up.")),
                            Err(why) => {
                                warn!("Error deserializing voice event: {:?}", why);

                                Err(Error::Json(why))
                            },
                        },
                        None => Ok(()),
                    })
            })
            .map_err(|e| {
                warn!("[voice] {}", e);

                ()
            })
    );

    context.spawn(move |_|
        udp.map_err(Error::from)
            .until(close_reader2.map(|v| *v))
            .for_each(|voice_frame|
                tx_clone.unbounded_send(ReceiverStatus::Udp(voice_frame))
                    .map_err(|e| Error::FutureMpsc("[voice] UDP event receiver hung up."))
            )
            .map_err(|e| {
                warn!("[voice] {}", e);

                ()
            })
    );

    ListenerItems {
        close_sender,
        rx,
        rx_pong,
    }
}
