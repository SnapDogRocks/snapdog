const express = require('express');
const http = require('http');
const WebSocket = require('ws');
const path = require('path');
const rateLimit = require('express-rate-limit');

const app = express();
const server = http.createServer(app);
const wss = new WebSocket.Server({ noServer: true });

// Trust first proxy for correct client IP detection under Render/Cloudflare
app.set('trust proxy', 1);

const limiter = rateLimit({
  windowMs: 60 * 1000, // 1 minute
  limit: 100, // Limit each IP to 100 requests per `window`
  standardHeaders: 'draft-7', // Use RFC 9421 RateLimit headers
  legacyHeaders: false, // Disable the `X-RateLimit-*` headers
  message: { error: 'Too many requests, please try again later.' },
});

// Apply the rate limiting middleware to all requests
app.use(limiter);

app.use(express.json({ strict: false }));
// Serve Next.js static files from 'public' directory
app.use(express.static(path.join(__dirname, 'public')));

// ── Mock Unsplash Images for Cover Art ────────────────────────
// Using specific, aesthetic Unsplash images for our playlists and radio stations
const COVERS = {
  lofi: 'https://images.unsplash.com/photo-1518495973542-4542c06a5843?w=1024&h=1024&auto=format&fit=crop&q=80', // cozy sunbeam/room
  synth: 'https://images.unsplash.com/photo-1618005182384-a83a8bd57fbe?w=1024&h=1024&auto=format&fit=crop&q=80', // abstract neon wave
  jazz: 'https://images.unsplash.com/photo-1511192336575-5a79af67a629?w=1024&h=1024&auto=format&fit=crop&q=80', // neon jazz sax/club
  focus: 'https://images.unsplash.com/photo-1447752875215-b2761acb3c5d?w=1024&h=1024&auto=format&fit=crop&q=80', // misty forest road
  acoustic: 'https://images.unsplash.com/photo-1504280390367-361c6d9f38f4?w=1024&h=1024&auto=format&fit=crop&q=80', // bonfire under stars
  cyber: 'https://images.unsplash.com/photo-1508739773434-c26b3d09e071?w=1024&h=1024&auto=format&fit=crop&q=80', // neon wet city street
  classical: 'https://images.unsplash.com/photo-1520523839897-bd0b52f945a0?w=1024&h=1024&auto=format&fit=crop&q=80', // grand piano keys
  summer: 'https://images.unsplash.com/photo-1507525428034-b723cf961d3e?w=1024&h=1024&auto=format&fit=crop&q=80', // tropical beach sunset
  metal: 'https://images.unsplash.com/photo-1508614589041-895b88991e3e?w=1024&h=1024&auto=format&fit=crop&q=80', // electric guitar wall
  midnight: 'https://images.unsplash.com/photo-1492496913980-50134c307287?w=1024&h=1024&auto=format&fit=crop&q=80', // rainy window view
};

// ── Mock Playlists & Tracks (Including Radio Playlist 0) ──────
const PLAYLISTS = [
  {
    id: 0,
    name: 'Radio',
    cover_art: '/assets/radio-cover.png',
    tracks: [
      { id: 'radio_0', title: 'Groove Salad (SomaFM)', artist: 'Radio', album: 'Ambient/Chill Beats', duration: 0, url: 'https://ice1.somafm.com/groovesalad-128-mp3' },
      { id: 'radio_1', title: 'Lofi Girl Radio', artist: 'Radio', album: 'Focus & Study Beats', duration: 0, url: 'https://lofigirl.com/' },
      { id: 'radio_2', title: 'BBC Radio 6 Music', artist: 'Radio', album: 'Alternative & Indie', duration: 0, url: 'https://stream.live.vc.bbcmedia.co.uk/bbc_6music' },
      { id: 'radio_3', title: 'KEXP Seattle', artist: 'Radio', album: 'Where the Music Matters', duration: 0, url: 'https://kexp-mp3-128.streamguys1.com/kexp128.mp3' },
      { id: 'radio_4', title: 'FIP Radio (Paris)', artist: 'Radio', album: 'Eclectic Radio France', duration: 0, url: 'https://stream.radiofrance.fr/fip/fip.m3u8' },
      { id: 'radio_5', title: 'Jazz24 (Seattle)', artist: 'Radio', album: 'Classic & Modern Jazz', duration: 0, url: 'https://live.wshu.org/jazz24' },
      { id: 'radio_6', title: 'Ibiza Global Radio', artist: 'Radio', album: 'Electronic House', duration: 0, url: 'https://ibizaglobalradio.live-streams.nl/' },
      { id: 'radio_7', title: 'Rock Antenne', artist: 'Radio', album: 'Classic Rock & Metal', duration: 0, url: 'https://stream.rockantenne.de/rockantenne/stream/mp3' },
      { id: 'radio_8', title: 'Space Station Soma', artist: 'Radio', album: 'Spaced Out Ambient', duration: 0, url: 'https://ice1.somafm.com/spacestation-128-mp3' },
      { id: 'radio_9', title: 'Cinemix', artist: 'Radio', album: 'Soundtracks & Orchestral', duration: 0, url: 'https://cinemix.stationplaylist.com/g2.aac' },
    ]
  },
  {
    id: 1,
    name: 'Lofi Chill Study',
    cover_art: COVERS.lofi,
    tracks: [
      { id: 'p1_t1', title: 'Coffee & Chill', artist: 'Dreamy Sloth', album: 'Bedroom Beats', duration: 165 },
      { id: 'p1_t2', title: 'Rainy Day Cafe', artist: 'Pluviophile', album: 'Warm Brews', duration: 190 },
      { id: 'p1_t3', title: 'Blanket Fort', artist: 'Snuggle Collective', album: 'Cozy Tapes', duration: 140 },
      { id: 'p1_t4', title: 'Floating Dust', artist: 'Hazy Sunbeam', album: 'Bedroom Beats', duration: 180 },
      { id: 'p1_t5', title: 'Homework Helper', artist: 'Focus Flow', album: 'Cozy Tapes', duration: 155 },
    ]
  },
  {
    id: 2,
    name: 'Synthwave Retro Drive',
    cover_art: COVERS.synth,
    tracks: [
      { id: 'p2_t1', title: 'Grid Runner', artist: 'Neon Vector', album: 'Outrun 1988', duration: 220 },
      { id: 'p2_t2', title: 'Laser Horizon', artist: 'Cyber Cruise', album: 'Grid Runner', duration: 245 },
      { id: 'p2_t3', title: 'Sunset Silhouette', artist: 'Analog Kid', album: 'Outrun 1988', duration: 210 },
      { id: 'p2_t4', title: 'Chrome Hearts', artist: 'Vapor Wave', album: 'Dream Drive', duration: 230 },
      { id: 'p2_t5', title: 'Digital Highway', artist: 'Outrun Legend', album: 'Grid Runner', duration: 250 },
    ]
  },
  {
    id: 3,
    name: 'Smooth Coffeehouse Jazz',
    cover_art: COVERS.jazz,
    tracks: [
      { id: 'p3_t1', title: 'Blue Brew', artist: 'Vibe Quartet', album: 'Midnight Sessions', duration: 310 },
      { id: 'p3_t2', title: 'Velvet Saxophone', artist: 'Miles Ahead Trio', album: 'Smooth Coffeehouse Jazz', duration: 275 },
      { id: 'p3_t3', title: 'Rainy Alley Waltz', artist: 'Billie Keys', album: 'Midnight Sessions', duration: 290 },
      { id: 'p3_t4', title: 'Espresso Solo', artist: 'The Coffee Trio', album: 'Smooth Coffeehouse Jazz', duration: 240 },
      { id: 'p3_t5', title: 'Warm Brass', artist: 'Smooth Operators', album: 'Warm Brass EP', duration: 300 },
    ]
  },
  {
    id: 4,
    name: 'Deep Focus Ambient',
    cover_art: COVERS.focus,
    tracks: [
      { id: 'p4_t1', title: 'Misty Canopy', artist: 'Forest Echo', album: 'Green Noise', duration: 420 },
      { id: 'p4_t2', title: 'Static Clouds', artist: 'Binaural Drift', album: 'Theta Waves', duration: 380 },
      { id: 'p4_t3', title: 'Submerged Thoughts', artist: 'Deep Sea Drones', album: 'Green Noise', duration: 480 },
      { id: 'p4_t4', title: 'Stellar Wind', artist: 'Cosmic Drone', album: 'Solfeggio Frequencies', duration: 360 },
      { id: 'p4_t5', title: 'Restless Mind', artist: 'Quiet Brain', album: 'Theta Waves', duration: 410 },
    ]
  },
  {
    id: 5,
    name: 'Acoustic Campfire',
    cover_art: COVERS.acoustic,
    tracks: [
      { id: 'p5_t1', title: 'Pine Needle Path', artist: 'The Woodlander', album: 'Acoustic Campfire', duration: 195 },
      { id: 'p5_t2', title: 'Sparks Fly Upward', artist: 'Stargazer Folk', album: 'Fireside Sessions', duration: 215 },
      { id: 'p5_t3', title: 'Guitar in the Mist', artist: 'Mountain Whisper', album: 'Acoustic Campfire', duration: 180 },
      { id: 'p5_t4', title: 'Cabin Waltz', artist: 'Lake Cabin', album: 'Fireside Sessions', duration: 205 },
      { id: 'p5_t5', title: 'Fading Embers', artist: 'Driftwood', album: 'Driftwood Folk', duration: 190 },
    ]
  },
  {
    id: 6,
    name: 'Cyberpunk Underground',
    cover_art: COVERS.cyber,
    tracks: [
      { id: 'p6_t1', title: 'Neon Syndicate', artist: 'Hack0r', album: 'Cyberpunk Underground', duration: 260 },
      { id: 'p6_t2', title: 'Hologram Alley', artist: 'System Crash', album: 'Glitch City', duration: 285 },
      { id: 'p6_t3', title: 'Acid Rain', artist: 'Black Ice', album: 'Cyberpunk Underground', duration: 240 },
      { id: 'p6_t4', title: 'Chiba City Port', artist: 'Case & Molly', album: 'Neuromancers', duration: 270 },
      { id: 'p6_t5', title: 'Netrunner', artist: 'Proxy server', album: 'Glitch City', duration: 295 },
    ]
  },
  {
    id: 7,
    name: 'Classical Solitude',
    cover_art: COVERS.classical,
    tracks: [
      { id: 'p7_t1', title: 'Nocturne in G minor', artist: 'Frederic Mock', album: 'Romantic Solitude', duration: 290 },
      { id: 'p7_t2', title: 'Gymnopedie No. 1', artist: 'Erik Mock', album: 'Classical Solitude', duration: 200 },
      { id: 'p7_t3', title: 'Moonlight Mock', artist: 'Ludwig Mock', album: 'Romantic Solitude', duration: 380 },
      { id: 'p7_t4', title: 'Prelude in C Major', artist: 'Johann Sebastian Mock', album: 'Classical Solitude', duration: 150 },
      { id: 'p7_t5', title: 'Air on a G String', artist: 'Sebastian Strings', album: 'Classical Solitude', duration: 260 },
    ]
  },
  {
    id: 8,
    name: 'Summer Beach Vibes',
    cover_art: COVERS.summer,
    tracks: [
      { id: 'p8_t1', title: 'Salty Breeze', artist: 'Island Boy', album: 'Summer Beach Vibes', duration: 200 },
      { id: 'p8_t2', title: 'Tropic Shore', artist: 'Sun Chaser', album: 'Tropic Shore', duration: 185 },
      { id: 'p8_t3', title: 'Coconut Groove', artist: 'Cabana Beats', album: 'Summer Beach Vibes', duration: 195 },
      { id: 'p8_t4', title: 'Golden Hour', artist: 'Sunset Cruise', album: 'Sun Chaser', duration: 215 },
      { id: 'p8_t5', title: 'Tide Pool', artist: 'Swell', album: 'Tropic Shore', duration: 175 },
    ]
  },
  {
    id: 9,
    name: 'Heavy Metal Workout',
    cover_art: COVERS.metal,
    tracks: [
      { id: 'p9_t1', title: 'Iron Grind', artist: 'Thrash Master', album: 'Heavy Metal Workout', duration: 250 },
      { id: 'p9_t2', title: 'Doom Amplifier', artist: 'Fuzz Lord', album: 'Riffs of Doom', duration: 290 },
      { id: 'p9_t3', title: 'Anvil Strike', artist: 'Forge Fire', album: 'Heavy Metal Workout', duration: 235 },
      { id: 'p9_t4', title: 'Double Bass Storm', artist: 'Slayer of Mocks', album: 'Riffs of Doom', duration: 275 },
      { id: 'p9_t5', title: 'Voltage Shred', artist: 'Circuit Breaker', album: 'Voltage Shred', duration: 245 },
    ]
  },
  {
    id: 10,
    name: 'Midnight Melancholy',
    cover_art: COVERS.midnight,
    tracks: [
      { id: 'p10_t1', title: 'Quiet Streets', artist: 'Sodden Soul', album: 'Midnight Melancholy', duration: 210 },
      { id: 'p10_t2', title: 'Headlight Glow', artist: 'Foggy Glass', album: 'Wet Pavement', duration: 230 },
      { id: 'p10_t3', title: 'Neon Reflection', artist: 'Night Owl', album: 'Midnight Melancholy', duration: 245 },
      { id: 'p10_t4', title: 'Cold Coffee', artist: 'Wasted Hours', album: 'Wet Pavement', duration: 195 },
      { id: 'p10_t5', title: 'Unanswered Call', artist: 'Hollow Tone', album: 'Midnight Melancholy', duration: 225 },
    ]
  }
];

// Helper to calculate total song counts and durations for playlist responses
const plInfoList = PLAYLISTS.map((pl) => ({
  id: pl.id,
  name: pl.name,
  song_count: pl.tracks.length,
  duration: pl.tracks.reduce((acc, t) => acc + t.duration, 0),
  cover_art: pl.id === 0 ? '/assets/radio-cover.png' : pl.cover_art,
}));

// ── Mock System State ─────────────────────────────────────────

let systemStatus = {
  version: 'v0.2.0-demo',
  zones: 3,
  clients: 6,
  radios: PLAYLISTS[0].tracks.length, // 10 radios
};

let versionInfo = {
  version: 'v0.2.0-demo',
  rust_version: 'rustc 1.78.0 (mock)',
  name: 'SnapDog Demo Server',
};

// Speakers list
const SPEAKERS = [
  'Custom Flat EQ',
  'Genelec 8030C',
  'Sonos One (Gen 2)',
  'JBL Control 1 Pro',
  'KEF Q150 Bookcase',
];

// Initial mock EQ bands (10 bands)
const defaultEqBands = [
  { filter_type: 'low_shelf', frequency: 31, gain: 0, q: 0.7 },
  { filter_type: 'peaking', frequency: 62, gain: 0, q: 1.0 },
  { filter_type: 'peaking', frequency: 125, gain: 0, q: 1.0 },
  { filter_type: 'peaking', frequency: 250, gain: 0, q: 1.0 },
  { filter_type: 'peaking', frequency: 500, gain: 0, q: 1.0 },
  { filter_type: 'peaking', frequency: 1000, gain: 0, q: 1.0 },
  { filter_type: 'peaking', frequency: 2000, gain: 0, q: 1.0 },
  { filter_type: 'peaking', frequency: 4000, gain: 0, q: 1.0 },
  { filter_type: 'peaking', frequency: 8000, gain: 0, q: 1.0 },
  { filter_type: 'high_shelf', frequency: 16000, gain: 0, q: 0.7 },
];

const PRESET_GAINS = {
  flat: [0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
  bass_boost: [6, 6, 5, 3, 1, 0, 0, 0, 0, 0],
  treble_boost: [0, 0, 0, 0, 0, 1, 2, 4, 5, 6],
  vocal: [-3, -2, -2, -1, 1, 3, 4, 4, 2, 0],
  rock: [4, 4, 3, -1, -2, -1, 1, 3, 4, 4],
  jazz: [3, 3, 2, 1, 2, -1, -1, 0, 1, 2],
  classical: [4, 4, 3, 2, 2, 0, -1, -1, 1, 3],
  electronic: [5, 5, 4, 0, -2, 2, -1, 1, 3, 5],
  loudness: [6, 6, 4, 0, -2, -3, -2, 0, 3, 5],
  late_night: [-4, -3, -2, 0, 1, 2, 2, 1, -1, -3]
};

const SPEAKER_PROFILES = {
  'Custom Flat EQ': [
    { filter_type: 'low_shelf', frequency: 31, gain: 0, q: 0.7 },
    { filter_type: 'peaking', frequency: 62, gain: 0, q: 1.0 },
    { filter_type: 'peaking', frequency: 125, gain: 0, q: 1.0 },
    { filter_type: 'peaking', frequency: 250, gain: 0, q: 1.0 },
    { filter_type: 'peaking', frequency: 500, gain: 0, q: 1.0 },
    { filter_type: 'peaking', frequency: 1000, gain: 0, q: 1.0 },
    { filter_type: 'peaking', frequency: 2000, gain: 0, q: 1.0 },
    { filter_type: 'peaking', frequency: 4000, gain: 0, q: 1.0 },
    { filter_type: 'peaking', frequency: 8000, gain: 0, q: 1.0 },
    { filter_type: 'high_shelf', frequency: 16000, gain: 0, q: 0.7 }
  ],
  'Genelec 8030C': [
    { filter_type: 'low_shelf', frequency: 31, gain: -4.0, q: 0.7 },
    { filter_type: 'peaking', frequency: 62, gain: -2.0, q: 1.0 },
    { filter_type: 'peaking', frequency: 125, gain: 1.5, q: 1.0 },
    { filter_type: 'peaking', frequency: 250, gain: -0.8, q: 1.0 },
    { filter_type: 'peaking', frequency: 500, gain: 0.5, q: 1.0 },
    { filter_type: 'peaking', frequency: 1000, gain: -0.3, q: 1.0 },
    { filter_type: 'peaking', frequency: 2000, gain: 1.2, q: 1.0 },
    { filter_type: 'peaking', frequency: 4000, gain: -0.5, q: 1.0 },
    { filter_type: 'peaking', frequency: 8000, gain: 0.8, q: 1.0 },
    { filter_type: 'high_shelf', frequency: 16000, gain: -1.0, q: 0.7 }
  ],
  'Sonos One (Gen 2)': [
    { filter_type: 'low_shelf', frequency: 31, gain: 5.0, q: 0.7 },
    { filter_type: 'peaking', frequency: 62, gain: 3.5, q: 1.0 },
    { filter_type: 'peaking', frequency: 125, gain: -2.0, q: 1.0 },
    { filter_type: 'peaking', frequency: 250, gain: 1.0, q: 1.0 },
    { filter_type: 'peaking', frequency: 500, gain: -1.5, q: 1.0 },
    { filter_type: 'peaking', frequency: 1000, gain: 2.5, q: 1.0 },
    { filter_type: 'peaking', frequency: 2000, gain: -1.0, q: 1.0 },
    { filter_type: 'peaking', frequency: 4000, gain: 1.8, q: 1.0 },
    { filter_type: 'peaking', frequency: 8000, gain: -2.0, q: 1.0 },
    { filter_type: 'high_shelf', frequency: 16000, gain: 3.0, q: 0.7 }
  ],
  'JBL Control 1 Pro': [
    { filter_type: 'low_shelf', frequency: 31, gain: -8.0, q: 0.7 },
    { filter_type: 'peaking', frequency: 62, gain: -5.0, q: 1.0 },
    { filter_type: 'peaking', frequency: 125, gain: 3.0, q: 1.0 },
    { filter_type: 'peaking', frequency: 250, gain: -2.5, q: 1.0 },
    { filter_type: 'peaking', frequency: 500, gain: 1.8, q: 1.0 },
    { filter_type: 'peaking', frequency: 1000, gain: -3.5, q: 1.0 },
    { filter_type: 'peaking', frequency: 2000, gain: 2.0, q: 1.0 },
    { filter_type: 'peaking', frequency: 4000, gain: -1.5, q: 1.0 },
    { filter_type: 'peaking', frequency: 8000, gain: 4.0, q: 1.0 },
    { filter_type: 'high_shelf', frequency: 16000, gain: 6.0, q: 0.7 }
  ],
  'KEF Q150 Bookcase': [
    { filter_type: 'low_shelf', frequency: 31, gain: -1.5, q: 0.7 },
    { filter_type: 'peaking', frequency: 62, gain: 1.5, q: 1.0 },
    { filter_type: 'peaking', frequency: 125, gain: -1.0, q: 1.0 },
    { filter_type: 'peaking', frequency: 250, gain: 0.8, q: 1.0 },
    { filter_type: 'peaking', frequency: 500, gain: -1.2, q: 1.0 },
    { filter_type: 'peaking', frequency: 1000, gain: 1.5, q: 1.0 },
    { filter_type: 'peaking', frequency: 2000, gain: -0.5, q: 1.0 },
    { filter_type: 'peaking', frequency: 4000, gain: 1.0, q: 1.0 },
    { filter_type: 'peaking', frequency: 8000, gain: -0.5, q: 1.0 },
    { filter_type: 'high_shelf', frequency: 16000, gain: 1.0, q: 0.7 }
  ]
};

const mockEqConfig = (enabled = false, preset = null, bands = null) => {
  let activeBands = bands ? JSON.parse(JSON.stringify(bands)) : JSON.parse(JSON.stringify(defaultEqBands));
  
  if (preset) {
    if (preset.startsWith('spinorama:')) {
      const spName = preset.slice('spinorama:'.length);
      if (SPEAKER_PROFILES[spName]) {
        activeBands = JSON.parse(JSON.stringify(SPEAKER_PROFILES[spName]));
      }
    } else if (PRESET_GAINS[preset]) {
      const gains = PRESET_GAINS[preset];
      activeBands.forEach((b, idx) => {
        if (gains[idx] !== undefined) b.gain = gains[idx];
      });
    }
  }
  
  return {
    enabled,
    bands: activeBands,
    preset,
  };
};

// Zones state
const zones = {
  1: {
    index: 1,
    name: 'Living Room',
    icon: 'sofa',
    volume: 65,
    muted: false,
    playback: 'playing',
    source: 'airplay',
    shuffle: false,
    repeat: 'off',
    presence: false,
    presence_enabled: true,
    presence_timer_active: false,
    eq: mockEqConfig(true),
    track_index: null,
    playlist_id: null,
    position_ms: 32000,
    airplay_meta: {
      title: 'AirPlay Audio Stream',
      artist: "Fabian's iPhone",
      album: 'AirPlay Connection',
      cover_url: 'https://images.unsplash.com/photo-1511671782779-c97d3d27a1d4?w=1024&h=1024&auto=format&fit=crop&q=80', // micro/headphone
      duration_ms: 0,
      seekable: false,
    }
  },
  2: {
    index: 2,
    name: 'Kitchen',
    icon: 'chef-hat',
    volume: 40,
    muted: false,
    playback: 'playing',
    source: 'radio',
    shuffle: false,
    repeat: 'off',
    presence: false,
    presence_enabled: true,
    presence_timer_active: false,
    eq: mockEqConfig(false),
    track_index: 0, // SomaFM Groove Salad
    playlist_id: 0, // Radio playlist
    position_ms: 45000,
  },
  3: {
    index: 3,
    name: 'Patio',
    icon: 'tree',
    volume: 50,
    muted: false,
    playback: 'playing',
    source: 'subsonic_playlist',
    shuffle: false,
    repeat: 'playlist',
    presence: false,
    presence_enabled: true,
    presence_timer_active: false,
    eq: mockEqConfig(false),
    track_index: 0,
    playlist_id: 1, // Lofi Chill
    position_ms: 12000,
  }
};

// Clients state
const clients = {
  1: {
    index: 1,
    name: 'Living Room Gallery',
    mac: '00:11:22:33:44:55',
    zone_index: 1,
    icon: 'speaker-center',
    volume: 75,
    max_volume: 100,
    muted: false,
    connected: true,
    is_snapdog: true,
    latency: 0,
    speaker: 'Genelec 8030C',
    eq: mockEqConfig(true, 'spinorama:Genelec 8030C'),
  },
  2: {
    index: 2,
    name: 'Living Room Sofa',
    mac: '00:11:22:33:44:56',
    zone_index: 1,
    icon: 'speaker-center',
    volume: 75,
    max_volume: 100,
    muted: false,
    connected: true,
    is_snapdog: true,
    latency: 0,
    speaker: 'Genelec 8030C',
    eq: mockEqConfig(true, 'spinorama:Genelec 8030C'),
  },
  3: {
    index: 3,
    name: 'Kitchen Ceiling',
    mac: '00:11:22:33:44:57',
    zone_index: 2,
    icon: 'speaker-center',
    volume: 50,
    max_volume: 85,
    muted: false,
    connected: true,
    is_snapdog: true,
    latency: 20,
    speaker: 'Sonos One (Gen 2)',
    eq: mockEqConfig(false, 'spinorama:Sonos One (Gen 2)'),
  },
  4: {
    index: 4,
    name: 'Patio Deck',
    mac: '00:11:22:33:44:58',
    zone_index: 3,
    icon: 'speaker-outdoor',
    volume: 60,
    max_volume: 100,
    muted: false,
    connected: true,
    is_snapdog: true,
    latency: 0,
    speaker: 'Custom Flat EQ',
    eq: mockEqConfig(false, 'spinorama:Custom Flat EQ'),
  },
  5: {
    index: 5,
    name: 'Kitchen Island',
    mac: '00:11:22:33:44:5a',
    zone_index: 2,
    icon: 'speaker-center',
    volume: 50,
    max_volume: 85,
    muted: false,
    connected: true,
    is_snapdog: true,
    latency: 20,
    speaker: 'Sonos One (Gen 2)',
    eq: mockEqConfig(false, 'spinorama:Sonos One (Gen 2)'),
  },
  6: {
    index: 6,
    name: 'Poolside',
    mac: '00:11:22:33:44:5b',
    zone_index: 3,
    icon: 'speaker-outdoor',
    volume: 60,
    max_volume: 100,
    muted: false,
    connected: true,
    is_snapdog: true,
    latency: 0,
    speaker: 'Custom Flat EQ',
    eq: mockEqConfig(false, 'spinorama:Custom Flat EQ'),
  }
};

let knxProgrammingMode = false;

// ── WebSocket Helper Functions ───────────────────────────────

function broadcast(notification) {
  const payload = JSON.stringify(notification);
  wss.clients.forEach((client) => {
    if (client.readyState === WebSocket.OPEN) {
      client.send(payload);
    }
  });
}

function getTrackMetadata(zoneId) {
  const zone = zones[zoneId];
  if (!zone) return null;

  if (zone.source === 'idle') {
    return null;
  }

  if (zone.source === 'airplay') {
    return {
      title: zone.airplay_meta.title,
      artist: zone.airplay_meta.artist,
      album: zone.airplay_meta.album,
      album_artist: zone.airplay_meta.artist,
      genre: 'AirPlay',
      year: new Date().getFullYear(),
      track_number: 1,
      disc_number: 1,
      duration_ms: zone.airplay_meta.duration_ms,
      position_ms: zone.position_ms,
      seekable: zone.airplay_meta.seekable,
      bitrate_kbps: 256,
      content_type: 'audio/alac',
      sample_rate: 44100,
      source: 'AirPlay',
      cover_url: zone.airplay_meta.cover_url,
      playlist_index: null,
      playlist_name: null,
      playlist_total: null,
      playlist_track_index: null,
      playlist_track_count: null,
      can_playlist_next: false,
      can_playlist_prev: false,
      can_next: false,
      can_prev: false,
    };
  }

  // Radio stations source
  if (zone.source === 'radio' && zone.track_index !== null) {
    const pl = PLAYLISTS.find(p => p.id === 0);
    if (pl) {
      const track = pl.tracks[zone.track_index];
      if (track) {
        return {
          title: track.title,
          artist: track.artist,
          album: track.album,
          album_artist: track.artist,
          genre: 'Internet Radio',
          year: new Date().getFullYear(),
          track_number: zone.track_index + 1,
          disc_number: 1,
          duration_ms: 0, // 0 means stream/infinite
          position_ms: zone.position_ms,
          seekable: false,
          bitrate_kbps: 128,
          content_type: 'audio/mp3',
          sample_rate: 44100,
          source: 'Radio',
          cover_url: `/api/v1/media/playlists/0/tracks/${zone.track_index}/cover`,
          playlist_index: 0,
          playlist_name: 'Radio',
          playlist_total: pl.tracks.length,
          playlist_track_index: zone.track_index,
          playlist_track_count: pl.tracks.length,
          can_playlist_next: zone.track_index < pl.tracks.length - 1,
          can_playlist_prev: zone.track_index > 0,
          can_next: false,
          can_prev: false,
        };
      }
    }
  }

  // Subsonic playlists (1+)
  if (zone.playlist_id !== null && zone.track_index !== null) {
    const pl = PLAYLISTS.find(p => p.id === zone.playlist_id);
    if (pl) {
      const track = pl.tracks[zone.track_index];
      if (track) {
        return {
          title: track.title,
          artist: track.artist,
          album: track.album,
          album_artist: track.artist,
          genre: 'Vocal / Instrumental',
          year: 2026,
          track_number: zone.track_index + 1,
          disc_number: 1,
          duration_ms: track.duration * 1000,
          position_ms: zone.position_ms,
          seekable: true,
          bitrate_kbps: 320,
          content_type: 'audio/mp3',
          sample_rate: 48000,
          source: 'Subsonic',
          cover_url: pl.cover_art,
          playlist_index: pl.id,
          playlist_name: pl.name,
          playlist_total: pl.tracks.length,
          playlist_track_index: zone.track_index,
          playlist_track_count: pl.tracks.length,
          can_playlist_next: zone.track_index < pl.tracks.length - 1,
          can_playlist_prev: zone.track_index > 0,
          can_next: zone.track_index < pl.tracks.length - 1,
          can_prev: zone.track_index > 0,
        };
      }
    }
  }

  return null;
}

// Generate the full zone notification payload for client updates
function buildWsZoneChanged(zoneId) {
  const zone = zones[zoneId];
  const meta = getTrackMetadata(zoneId);
  return {
    type: 'zone_changed',
    zone: zoneId,
    playback: zone.playback,
    source: zone.source,
    shuffle: zone.shuffle,
    repeat: zone.repeat,
    title: meta ? meta.title : '',
    artist: meta ? meta.artist : '',
    album: meta ? meta.album : '',
    album_artist: meta ? meta.album_artist : null,
    genre: meta ? meta.genre : null,
    year: meta ? meta.year : null,
    track_number: meta ? meta.track_number : null,
    disc_number: meta ? meta.disc_number : null,
    duration_ms: meta ? meta.duration_ms : 0,
    position_ms: zone.position_ms,
    seekable: meta ? meta.seekable : false,
    cover_url: meta ? meta.cover_url : null,
    bitrate_kbps: meta ? meta.bitrate_kbps : null,
    content_type: meta ? meta.content_type : null,
    track_index: meta ? meta.playlist_track_index : null,
    track_count: meta ? meta.playlist_track_count : null,
    playlist: meta ? meta.playlist_index : null,
    playlist_name: meta ? meta.playlist_name : null,
    playlist_total: meta ? meta.playlist_total : null,
    can_playlist_next: meta ? meta.can_playlist_next : false,
    can_playlist_prev: meta ? meta.can_playlist_prev : false,
    can_next: meta ? meta.can_next : false,
    can_prev: meta ? meta.can_prev : false,
    volume: zone.volume,
    muted: zone.muted,
  };
}

function buildWsClientStateChanged(clientId) {
  const client = clients[clientId];
  return {
    type: 'client_state_changed',
    client: clientId,
    volume: client.volume,
    muted: client.muted,
    connected: client.connected,
    zone: client.zone_index,
    is_snapdog: client.is_snapdog,
  };
}

// ── Playback Simulation Loop (1s ticker) ──────────────────────

setInterval(() => {
  Object.keys(zones).forEach((id) => {
    const zone = zones[id];
    if (zone.playback === 'playing') {
      const meta = getTrackMetadata(zone.index);
      if (zone.source === 'airplay' || zone.source === 'radio') {
        // AirPlay & Radio streams just tick up indefinitely (live streams)
        zone.position_ms += 1000;
        broadcast({
          type: 'zone_progress',
          zone: zone.index,
          position_ms: zone.position_ms,
          duration_ms: 0,
        });
      } else if (meta && meta.duration_ms > 0) {
        zone.position_ms += 1000;
        if (zone.position_ms >= meta.duration_ms) {
          // Track ended - advance to next or repeat
          zone.position_ms = 0;
          const pl = PLAYLISTS.find(p => p.id === zone.playlist_id);
          if (pl) {
            if (zone.repeat === 'track') {
              // Stay on current track
            } else if (zone.track_index < pl.tracks.length - 1) {
              zone.track_index++;
            } else if (zone.repeat === 'playlist') {
              zone.track_index = 0;
            } else {
              zone.playback = 'stopped';
              zone.position_ms = 0;
            }
          }
          broadcast(buildWsZoneChanged(zone.index));
        } else {
          // Normal progress update
          broadcast({
            type: 'zone_progress',
            zone: zone.index,
            position_ms: zone.position_ms,
            duration_ms: meta.duration_ms,
          });
        }
      }
    }
  });
}, 1000);

// ── Express REST API Routes ──────────────────────────────────

// Health Checks
app.get('/health', (req, res) => res.json({ status: 'healthy', zones: Object.keys(zones).length, clients: Object.keys(clients).length }));
app.get('/health/ready', (req, res) => res.send('ready'));
app.get('/health/live', (req, res) => res.send('live'));

// System
app.get('/api/v1/system/status', (req, res) => res.json(systemStatus));
app.get('/api/v1/system/version', (req, res) => res.json(versionInfo));

// Speakers
app.get('/api/v1/speakers', (req, res) => res.json(SPEAKERS));

// KNX
app.get('/api/v1/knx/programming-mode', (req, res) => res.json(knxProgrammingMode));
app.put('/api/v1/knx/programming-mode', (req, res) => {
  knxProgrammingMode = !!req.body;
  res.json(knxProgrammingMode);
});

// Media / Playlists
app.get('/api/v1/media/playlists', (req, res) => res.json(plInfoList));
app.get('/api/v1/media/playlists/:id', (req, res) => {
  const pl = plInfoList.find(p => p.id === parseInt(req.params.id));
  if (!pl) return res.status(404).json({ error: 'Playlist not found' });
  res.json(pl);
});
app.get('/api/v1/media/playlists/:id/tracks', (req, res) => {
  const pl = PLAYLISTS.find(p => p.id === parseInt(req.params.id));
  if (!pl) return res.status(404).json({ error: 'Playlist not found' });
  const tracksInfo = pl.tracks.map((t, idx) => ({
    id: t.id,
    title: t.title,
    artist: t.artist,
    album: t.album,
    duration: t.duration,
    track: idx + 1,
    cover_art: pl.id === 0 ? `/api/v1/media/playlists/0/tracks/${idx}/cover` : pl.cover_art,
  }));
  res.json(tracksInfo);
});
app.get('/api/v1/media/playlists/:id/tracks/:idx', (req, res) => {
  const pl = PLAYLISTS.find(p => p.id === parseInt(req.params.id));
  if (!pl) return res.status(404).json({ error: 'Playlist not found' });
  const idx = parseInt(req.params.idx);
  const track = pl.tracks[idx];
  if (!track) return res.status(404).json({ error: 'Track not found' });
  res.json({
    id: track.id,
    title: track.title,
    artist: track.artist,
    album: track.album,
    duration: track.duration,
    track: idx + 1,
    cover_art: pl.id === 0 ? `/api/v1/media/playlists/0/tracks/${idx}/cover` : pl.cover_art,
  });
});

// Cover art redirection
app.get('/api/v1/media/playlists/:id/cover', (req, res) => {
  const plId = parseInt(req.params.id);
  if (plId === 0) {
    res.redirect('/assets/radio-cover.png');
  } else {
    const pl = PLAYLISTS.find(p => p.id === plId);
    if (pl && pl.cover_art) {
      res.redirect(pl.cover_art);
    } else {
      res.status(404).send('Cover not found');
    }
  }
});

app.get('/api/v1/media/playlists/:id/tracks/:idx/cover', (req, res) => {
  const plId = parseInt(req.params.id);
  const trackIdx = parseInt(req.params.idx);
  
  if (plId === 0) {
    const radioCovers = [
      COVERS.lofi,       // Groove Salad
      COVERS.midnight,   // Lofi Girl
      COVERS.acoustic,   // BBC 6 Music
      COVERS.jazz,       // KEXP
      COVERS.cyber,      // FIP
      COVERS.classical,  // Jazz24
      COVERS.summer,     // Ibiza Global
      COVERS.metal,      // Rock Antenne
      COVERS.synth,      // Space Station Soma
      COVERS.focus,      // Cinemix
    ];
    const cover = radioCovers[trackIdx] || COVERS.lofi;
    res.redirect(cover);
  } else {
    const pl = PLAYLISTS.find(p => p.id === plId);
    if (pl && pl.tracks[trackIdx]) {
      res.redirect(pl.cover_art);
    } else {
      res.status(404).send('Cover not found');
    }
  }
});

// Zones REST
app.get('/api/v1/zones', (req, res) => {
  res.json(Object.values(zones).map(z => ({
    index: z.index,
    name: z.name,
    icon: z.icon,
    volume: z.volume,
    muted: z.muted,
    playback: z.playback,
    source: z.source,
    shuffle: z.shuffle,
    repeat: z.repeat,
    presence: z.presence,
    presence_enabled: z.presence_enabled,
    presence_timer_active: z.presence_timer_active,
  })));
});

app.get('/api/v1/zones/count', (req, res) => res.json(Object.keys(zones).length));

app.get('/api/v1/zones/:id', (req, res) => {
  const z = zones[req.params.id];
  if (!z) return res.status(404).json({ error: 'Zone not found' });
  res.json({
    index: z.index,
    name: z.name,
    icon: z.icon,
    volume: z.volume,
    muted: z.muted,
    playback: z.playback,
    source: z.source,
    shuffle: z.shuffle,
    repeat: z.repeat,
    presence: z.presence,
    presence_enabled: z.presence_enabled,
    presence_timer_active: z.presence_timer_active,
  });
});

app.get('/api/v1/zones/:id/name', (req, res) => {
  const z = zones[req.params.id];
  if (!z) return res.status(404).json({ error: 'Zone not found' });
  res.json(z.name);
});

app.get('/api/v1/zones/:id/icon', (req, res) => {
  const z = zones[req.params.id];
  if (!z) return res.status(404).json({ error: 'Zone not found' });
  res.json(z.icon);
});

app.get('/api/v1/zones/:id/playback', (req, res) => {
  const z = zones[req.params.id];
  if (!z) return res.status(404).json({ error: 'Zone not found' });
  res.json(z.playback);
});

app.get('/api/v1/zones/:id/clients', (req, res) => {
  const zoneId = parseInt(req.params.id);
  const clientIds = Object.values(clients)
    .filter(c => c.zone_index === zoneId)
    .map(c => c.index);
  res.json(clientIds);
});

// Zone Volume
app.get('/api/v1/zones/:id/volume', (req, res) => {
  const z = zones[req.params.id];
  if (!z) return res.status(404).json({ error: 'Zone not found' });
  res.json(z.volume);
});

app.put('/api/v1/zones/:id/volume', (req, res) => {
  const z = zones[req.params.id];
  if (!z) return res.status(404).json({ error: 'Zone not found' });
  
  const v = req.body;
  if (typeof v === 'string') {
    if (v.startsWith('+')) {
      z.volume = Math.min(100, z.volume + parseInt(v.slice(1)));
    } else if (v.startsWith('-')) {
      z.volume = Math.max(0, z.volume - parseInt(v.slice(1)));
    } else {
      z.volume = parseInt(v) || 0;
    }
  } else if (typeof v === 'number') {
    z.volume = Math.max(0, Math.min(100, v));
  }
  
  Object.values(clients).forEach(c => {
    if (c.zone_index === z.index && c.connected) {
      c.volume = z.volume;
      broadcast(buildWsClientStateChanged(c.index));
    }
  });

  res.json(z.volume);
  broadcast({
    type: 'zone_volume_changed',
    zone: z.index,
    volume: z.volume,
    muted: z.muted,
  });
});

// Zone Mute
app.get('/api/v1/zones/:id/mute', (req, res) => {
  const z = zones[req.params.id];
  if (!z) return res.status(404).json({ error: 'Zone not found' });
  res.json(z.muted);
});

app.put('/api/v1/zones/:id/mute', (req, res) => {
  const z = zones[req.params.id];
  if (!z) return res.status(404).json({ error: 'Zone not found' });
  z.muted = !!req.body;
  
  Object.values(clients).forEach(c => {
    if (c.zone_index === z.index && c.connected) {
      c.muted = z.muted;
      broadcast(buildWsClientStateChanged(c.index));
    }
  });

  res.sendStatus(204);
  broadcast({
    type: 'zone_volume_changed',
    zone: z.index,
    volume: z.volume,
    muted: z.muted,
  });
});

app.post('/api/v1/zones/:id/mute/toggle', (req, res) => {
  const z = zones[req.params.id];
  if (!z) return res.status(404).json({ error: 'Zone not found' });
  z.muted = !z.muted;
  
  Object.values(clients).forEach(c => {
    if (c.zone_index === z.index && c.connected) {
      c.muted = z.muted;
      broadcast(buildWsClientStateChanged(c.index));
    }
  });

  res.sendStatus(204);
  broadcast({
    type: 'zone_volume_changed',
    zone: z.index,
    volume: z.volume,
    muted: z.muted,
  });
});

// Transport Controls
app.post('/api/v1/zones/:id/play', (req, res) => {
  const z = zones[req.params.id];
  if (!z) return res.status(404).json({ error: 'Zone not found' });
  if (z.source === 'idle') {
    z.source = 'subsonic_playlist';
    z.playlist_id = 1;
    z.track_index = 0;
  }
  z.playback = 'playing';
  res.sendStatus(204);
  broadcast(buildWsZoneChanged(z.index));
});

app.post('/api/v1/zones/:id/pause', (req, res) => {
  const z = zones[req.params.id];
  if (!z) return res.status(404).json({ error: 'Zone not found' });
  z.playback = 'paused';
  res.sendStatus(204);
  broadcast(buildWsZoneChanged(z.index));
});

app.post('/api/v1/zones/:id/stop', (req, res) => {
  const z = zones[req.params.id];
  if (!z) return res.status(404).json({ error: 'Zone not found' });
  z.playback = 'stopped';
  z.position_ms = 0;
  res.sendStatus(204);
  broadcast(buildWsZoneChanged(z.index));
});

app.post('/api/v1/zones/:id/next', (req, res) => {
  const z = zones[req.params.id];
  if (!z) return res.status(404).json({ error: 'Zone not found' });
  
  if (z.playlist_id !== null && z.track_index !== null) {
    const pl = PLAYLISTS.find(p => p.id === z.playlist_id);
    if (pl && z.track_index < pl.tracks.length - 1) {
      z.track_index++;
      z.position_ms = 0;
    }
  }
  res.sendStatus(204);
  broadcast(buildWsZoneChanged(z.index));
});

app.post('/api/v1/zones/:id/previous', (req, res) => {
  const z = zones[req.params.id];
  if (!z) return res.status(404).json({ error: 'Zone not found' });
  
  if (z.playlist_id !== null && z.track_index !== null) {
    if (z.track_index > 0) {
      z.track_index--;
      z.position_ms = 0;
    }
  }
  res.sendStatus(204);
  broadcast(buildWsZoneChanged(z.index));
});

// Zone Shuffle & Repeat
app.get('/api/v1/zones/:id/shuffle', (req, res) => {
  const z = zones[req.params.id];
  if (!z) return res.status(404).json({ error: 'Zone not found' });
  res.json(z.shuffle);
});

app.put('/api/v1/zones/:id/shuffle', (req, res) => {
  const z = zones[req.params.id];
  if (!z) return res.status(404).json({ error: 'Zone not found' });
  z.shuffle = !!req.body;
  res.sendStatus(204);
  broadcast(buildWsZoneChanged(z.index));
});

app.post('/api/v1/zones/:id/shuffle/toggle', (req, res) => {
  const z = zones[req.params.id];
  if (!z) return res.status(404).json({ error: 'Zone not found' });
  z.shuffle = !z.shuffle;
  res.sendStatus(204);
  broadcast(buildWsZoneChanged(z.index));
});

app.get('/api/v1/zones/:id/repeat', (req, res) => {
  const z = zones[req.params.id];
  if (!z) return res.status(404).json({ error: 'Zone not found' });
  res.json(z.repeat);
});

app.put('/api/v1/zones/:id/repeat', (req, res) => {
  const z = zones[req.params.id];
  if (!z) return res.status(404).json({ error: 'Zone not found' });
  z.repeat = req.body;
  res.sendStatus(204);
  broadcast(buildWsZoneChanged(z.index));
});

app.post('/api/v1/zones/:id/repeat/toggle', (req, res) => {
  const z = zones[req.params.id];
  if (!z) return res.status(404).json({ error: 'Zone not found' });
  const modes = ['off', 'track', 'playlist'];
  const idx = modes.indexOf(z.repeat);
  z.repeat = modes[(idx + 1) % modes.length];
  res.sendStatus(204);
  broadcast(buildWsZoneChanged(z.index));
});

// Zone Track Position
app.get('/api/v1/zones/:id/track/metadata', (req, res) => {
  const meta = getTrackMetadata(req.params.id);
  if (!meta) return res.status(404).json({ error: 'No track playing' });
  res.json(meta);
});

app.get('/api/v1/zones/:id/track/position', (req, res) => {
  const z = zones[req.params.id];
  if (!z) return res.status(404).json({ error: 'Zone not found' });
  res.json(z.position_ms);
});

app.put('/api/v1/zones/:id/track/position', (req, res) => {
  const z = zones[req.params.id];
  if (!z) return res.status(404).json({ error: 'Zone not found' });
  z.position_ms = req.body.position_ms || 0;
  res.sendStatus(204);
  const meta = getTrackMetadata(z.index);
  broadcast({
    type: 'zone_progress',
    zone: z.index,
    position_ms: z.position_ms,
    duration_ms: meta ? meta.duration_ms : 0,
  });
});

app.get('/api/v1/zones/:id/track/progress', (req, res) => {
  const z = zones[req.params.id];
  const meta = getTrackMetadata(req.params.id);
  if (!z || !meta || meta.duration_ms === 0) return res.json(0);
  res.json(z.position_ms / meta.duration_ms);
});

app.put('/api/v1/zones/:id/track/progress', (req, res) => {
  const z = zones[req.params.id];
  const meta = getTrackMetadata(req.params.id);
  if (!z || !meta || meta.duration_ms === 0) return res.sendStatus(400);
  z.position_ms = Math.round(req.body * meta.duration_ms);
  res.sendStatus(204);
  broadcast({
    type: 'zone_progress',
    zone: z.index,
    position_ms: z.position_ms,
    duration_ms: meta.duration_ms,
  });
});

// Zone Play Specifics
app.post('/api/v1/zones/:id/play/track', (req, res) => {
  const z = zones[req.params.id];
  if (!z) return res.status(404).json({ error: 'Zone not found' });
  const trackIdx = parseInt(req.body);
  if (z.playlist_id !== null) {
    const pl = PLAYLISTS.find(p => p.id === z.playlist_id);
    if (pl && pl.tracks[trackIdx]) {
      z.track_index = trackIdx;
      z.position_ms = 0;
      z.playback = 'playing';
      z.source = z.playlist_id === 0 ? 'radio' : 'subsonic_playlist';
    }
  }
  res.sendStatus(204);
  broadcast(buildWsZoneChanged(z.index));
});

app.post('/api/v1/zones/:id/play/url', (req, res) => {
  const z = zones[req.params.id];
  if (!z) return res.status(404).json({ error: 'Zone not found' });
  
  z.source = 'url';
  z.playlist_id = null;
  z.track_index = null;
  z.position_ms = 0;
  z.playback = 'playing';
  z.airplay_meta = {
    title: 'Custom URL Stream',
    artist: req.body,
    album: 'Web Stream',
    cover_url: COVERS.midnight,
    duration_ms: 0,
    seekable: false,
  };
  
  res.sendStatus(204);
  broadcast(buildWsZoneChanged(z.index));
});

app.post('/api/v1/zones/:id/play/playlist', (req, res) => {
  const z = zones[req.params.id];
  if (!z) return res.status(404).json({ error: 'Zone not found' });
  
  const plId = parseInt(req.body.id);
  const trackIdx = parseInt(req.body.track) || 0;
  const pl = PLAYLISTS.find(p => p.id === plId);
  if (pl) {
    z.source = plId === 0 ? 'radio' : 'subsonic_playlist';
    z.playlist_id = plId;
    z.track_index = trackIdx;
    z.position_ms = 0;
    z.playback = 'playing';
  }
  res.sendStatus(204);
  broadcast(buildWsZoneChanged(z.index));
});

app.post('/api/v1/zones/:zoneId/play/playlist/:playlistId/track', (req, res) => {
  const z = zones[req.params.zoneId];
  if (!z) return res.status(404).json({ error: 'Zone not found' });
  
  const plId = parseInt(req.params.playlistId);
  const trackIdx = parseInt(req.body);
  const pl = PLAYLISTS.find(p => p.id === plId);
  if (pl) {
    z.source = plId === 0 ? 'radio' : 'subsonic_playlist';
    z.playlist_id = plId;
    z.track_index = trackIdx;
    z.position_ms = 0;
    z.playback = 'playing';
  }
  res.sendStatus(204);
  broadcast(buildWsZoneChanged(z.index));
});

// Zone Playlist Navigation
app.get('/api/v1/zones/:id/playlist', (req, res) => {
  const z = zones[req.params.id];
  if (!z) return res.status(404).json({ error: 'Zone not found' });
  res.json(z.playlist_id);
});

app.put('/api/v1/zones/:id/playlist', (req, res) => {
  const z = zones[req.params.id];
  if (!z) return res.status(404).json({ error: 'Zone not found' });
  
  const plId = parseInt(req.body);
  const pl = PLAYLISTS.find(p => p.id === plId);
  if (pl) {
    z.playlist_id = plId;
    z.track_index = 0;
    z.position_ms = 0;
    z.source = plId === 0 ? 'radio' : 'subsonic_playlist';
  }
  res.sendStatus(204);
  broadcast(buildWsZoneChanged(z.index));
});

app.post('/api/v1/zones/:id/next/playlist', (req, res) => {
  const z = zones[req.params.id];
  if (!z) return res.status(404).json({ error: 'Zone not found' });
  
  if (z.playlist_id !== null) {
    const nextId = (z.playlist_id + 1) % PLAYLISTS.length;
    z.playlist_id = nextId;
    z.track_index = 0;
    z.position_ms = 0;
    z.source = nextId === 0 ? 'radio' : 'subsonic_playlist';
  }
  res.sendStatus(204);
  broadcast(buildWsZoneChanged(z.index));
});

app.post('/api/v1/zones/:id/previous/playlist', (req, res) => {
  const z = zones[req.params.id];
  if (!z) return res.status(404).json({ error: 'Zone not found' });
  
  if (z.playlist_id !== null) {
    const prevId = z.playlist_id === 0 ? PLAYLISTS.length - 1 : z.playlist_id - 1;
    z.playlist_id = prevId;
    z.track_index = 0;
    z.position_ms = 0;
    z.source = prevId === 0 ? 'radio' : 'subsonic_playlist';
  }
  res.sendStatus(204);
  broadcast(buildWsZoneChanged(z.index));
});

app.get('/api/v1/zones/:id/playlist/info', (req, res) => {
  const z = zones[req.params.id];
  if (!z) return res.status(404).json({ error: 'Zone not found' });
  if (z.playlist_id === null) {
    return res.json({ index: null, name: null, total: null, track_index: null, track_count: null });
  }
  const pl = PLAYLISTS.find(p => p.id === z.playlist_id);
  res.json({
    index: z.playlist_id,
    name: pl ? pl.name : null,
    total: pl ? pl.tracks.length : null,
    track_index: z.track_index,
    track_count: pl ? pl.tracks.length : null,
  });
});

// Zone EQ
app.get('/api/v1/zones/:id/eq', (req, res) => {
  const z = zones[req.params.id];
  if (!z) return res.status(404).json({ error: 'Zone not found' });
  res.json(z.eq);
});

app.put('/api/v1/zones/:id/eq', (req, res) => {
  const z = zones[req.params.id];
  if (!z) return res.status(404).json({ error: 'Zone not found' });
  z.eq.enabled = !!req.body.enabled;
  if (req.body.bands) z.eq.bands = req.body.bands;
  z.eq.preset = req.body.preset || null;
  res.json(z.eq);
  broadcast({
    type: 'zone_eq_changed',
    zone: z.index,
    enabled: z.eq.enabled,
    bands: z.eq.bands,
    preset: z.eq.preset,
  });
});

app.put('/api/v1/zones/:id/eq/:idx', (req, res) => {
  const z = zones[req.params.id];
  if (!z) return res.status(404).json({ error: 'Zone not found' });
  const bandIdx = parseInt(req.params.idx);
  if (z.eq.bands[bandIdx]) {
    z.eq.bands[bandIdx] = req.body;
  }
  res.json(z.eq);
  broadcast({
    type: 'zone_eq_changed',
    zone: z.index,
    enabled: z.eq.enabled,
    bands: z.eq.bands,
    preset: z.eq.preset,
  });
});

app.post('/api/v1/zones/:id/eq/preset', (req, res) => {
  const z = zones[req.params.id];
  if (!z) return res.status(404).json({ error: 'Zone not found' });
  
  const preset = typeof req.body === 'string' ? req.body : (req.body.preset || Object.keys(req.body)[0] || 'flat');
  
  z.eq = mockEqConfig(true, preset);
  res.json(z.eq);
  broadcast({
    type: 'zone_eq_changed',
    zone: z.index,
    enabled: z.eq.enabled,
    bands: z.eq.bands,
    preset: z.eq.preset,
  });
});

// ── Clients REST API ──────────────────────────────────────────

app.get('/api/v1/clients', (req, res) => {
  res.json(Object.values(clients));
});

app.get('/api/v1/clients/count', (req, res) => res.json(Object.keys(clients).length));

app.get('/api/v1/clients/:id', (req, res) => {
  const c = clients[req.params.id];
  if (!c) return res.status(404).json({ error: 'Client not found' });
  res.json(c);
});

// Client Volume
app.get('/api/v1/clients/:id/volume', (req, res) => {
  const c = clients[req.params.id];
  if (!c) return res.status(404).json({ error: 'Client not found' });
  res.json(c.volume);
});

app.put('/api/v1/clients/:id/volume', (req, res) => {
  const c = clients[req.params.id];
  if (!c) return res.status(404).json({ error: 'Client not found' });
  
  const v = req.body;
  if (typeof v === 'string') {
    if (v.startsWith('+')) {
      c.volume = Math.min(c.max_volume, c.volume + parseInt(v.slice(1)));
    } else if (v.startsWith('-')) {
      c.volume = Math.max(0, c.volume - parseInt(v.slice(1)));
    } else {
      c.volume = parseInt(v) || 0;
    }
  } else if (typeof v === 'number') {
    c.volume = Math.max(0, Math.min(c.max_volume, v));
  }
  
  res.json(c.volume);
  broadcast(buildWsClientStateChanged(c.index));
});

// Client Mute
app.get('/api/v1/clients/:id/mute', (req, res) => {
  const c = clients[req.params.id];
  if (!c) return res.status(404).json({ error: 'Client not found' });
  res.json(c.muted);
});

app.put('/api/v1/clients/:id/mute', (req, res) => {
  const c = clients[req.params.id];
  if (!c) return res.status(404).json({ error: 'Client not found' });
  c.muted = !!req.body;
  res.sendStatus(204);
  broadcast(buildWsClientStateChanged(c.index));
});

app.post('/api/v1/clients/:id/mute/toggle', (req, res) => {
  const c = clients[req.params.id];
  if (!c) return res.status(404).json({ error: 'Client not found' });
  c.muted = !c.muted;
  res.sendStatus(204);
  broadcast(buildWsClientStateChanged(c.index));
});

// Client Latency
app.get('/api/v1/clients/:id/latency', (req, res) => {
  const c = clients[req.params.id];
  if (!c) return res.status(404).json({ error: 'Client not found' });
  res.json(c.latency);
});

app.put('/api/v1/clients/:id/latency', (req, res) => {
  const c = clients[req.params.id];
  if (!c) return res.status(404).json({ error: 'Client not found' });
  c.latency = parseInt(req.body) || 0;
  res.sendStatus(204);
});

// Client Zone Mapping
app.get('/api/v1/clients/:id/zone', (req, res) => {
  const c = clients[req.params.id];
  if (!c) return res.status(404).json({ error: 'Client not found' });
  res.json(c.zone_index);
});

app.put('/api/v1/clients/:id/zone', (req, res) => {
  const c = clients[req.params.id];
  if (!c) return res.status(404).json({ error: 'Client not found' });
  const zoneId = parseInt(req.body);
  if (!zones[zoneId]) {
    return res.status(404).json({ error: 'Zone not found' });
  }
  c.zone_index = zoneId;
  res.json(c.zone_index);
  broadcast(buildWsClientStateChanged(c.index));
});

// Client Name
app.get('/api/v1/clients/:id/name', (req, res) => {
  const c = clients[req.params.id];
  if (!c) return res.status(404).json({ error: 'Client not found' });
  res.json(c.name);
});

app.put('/api/v1/clients/:id/name', (req, res) => {
  const c = clients[req.params.id];
  if (!c) return res.status(404).json({ error: 'Client not found' });
  c.name = String(req.body);
  res.sendStatus(204);
  broadcast(buildWsClientStateChanged(c.index));
});

// Client Icon & Connected
app.get('/api/v1/clients/:id/icon', (req, res) => {
  const c = clients[req.params.id];
  if (!c) return res.status(404).json({ error: 'Client not found' });
  res.json(c.icon);
});

app.get('/api/v1/clients/:id/connected', (req, res) => {
  const c = clients[req.params.id];
  if (!c) return res.status(404).json({ error: 'Client not found' });
  res.json(c.connected);
});

// Client EQ
app.get('/api/v1/clients/:id/eq', (req, res) => {
  const c = clients[req.params.id];
  if (!c) return res.status(404).json({ error: 'Client not found' });
  res.json(c.eq);
});

app.put('/api/v1/clients/:id/eq', (req, res) => {
  const c = clients[req.params.id];
  if (!c) return res.status(404).json({ error: 'Client not found' });
  c.eq.enabled = !!req.body.enabled;
  if (req.body.bands) c.eq.bands = req.body.bands;
  c.eq.preset = req.body.preset || null;
  res.json(c.eq);
});

app.put('/api/v1/clients/:id/eq/:idx', (req, res) => {
  const c = clients[req.params.id];
  if (!c) return res.status(404).json({ error: 'Client not found' });
  const bandIdx = parseInt(req.params.idx);
  if (c.eq.bands[bandIdx]) {
    c.eq.bands[bandIdx] = req.body;
  }
  res.json(c.eq);
});

app.post('/api/v1/clients/:id/eq/preset', (req, res) => {
  const c = clients[req.params.id];
  if (!c) return res.status(404).json({ error: 'Client not found' });
  
  const preset = typeof req.body === 'string' ? req.body : (req.body.preset || Object.keys(req.body)[0] || 'flat');
  c.eq = mockEqConfig(true, preset);
  res.json(c.eq);
});

// Client Speaker Profiles
app.get('/api/v1/clients/:id/speaker', (req, res) => {
  const c = clients[req.params.id];
  if (!c) return res.status(404).json({ error: 'Client not found' });
  
  if (c.speaker && c.speaker !== 'Custom Profile') {
    res.json(mockEqConfig(c.eq ? c.eq.enabled : true, `spinorama:${c.speaker}`));
  } else {
    res.json(c.eq || mockEqConfig(false, 'flat'));
  }
});

app.put('/api/v1/clients/:id/speaker', (req, res) => {
  const c = clients[req.params.id];
  if (!c) return res.status(404).json({ error: 'Client not found' });
  
  if (req.body.speaker) {
    c.speaker = req.body.speaker;
    c.eq = mockEqConfig(true, `spinorama:${c.speaker}`);
  } else if (req.body.custom) {
    c.speaker = 'Custom Profile';
    c.eq = req.body.custom;
  } else {
    c.speaker = null;
    c.eq = mockEqConfig(false, 'flat');
  }
  res.json(c.eq);
});

// Speaker Profile lookup
app.get('/api/v1/speakers/:name/profile', (req, res) => {
  const name = req.params.name;
  res.json(mockEqConfig(true, `spinorama:${name}`));
});

// Fallback: Catch-all for Next.js routing (client-side routing)
app.get('*', (req, res, next) => {
  if (req.path.startsWith('/api/') || req.path === '/health' || req.path.startsWith('/health/')) {
    return next();
  }
  res.sendFile(path.join(__dirname, 'public', 'index.html'));
});

// ── WebSocket Handler ─────────────────────────────────────────

wss.on('connection', (ws, req) => {
  // On connection, send initial zone states and client states to the new subscriber
  Object.keys(zones).forEach((id) => {
    ws.send(JSON.stringify(buildWsZoneChanged(parseInt(id))));
  });
  
  Object.keys(clients).forEach((id) => {
    ws.send(JSON.stringify(buildWsClientStateChanged(parseInt(id))));
  });

  ws.on('message', (message) => {
    try {
      const data = JSON.parse(message);
      if (data.action && data.zone) {
        console.log(`WS Command Received for zone ${data.zone}: ${data.action}`);
      }
    } catch (e) {
      /* ignore */
    }
  });
});

// HTTP Upgrade to WebSocket
server.on('upgrade', (request, socket, head) => {
  const pathname = new URL(request.url, `http://${request.headers.host}`).pathname;
  if (pathname === '/ws') {
    wss.handleUpgrade(request, socket, head, (ws) => {
      wss.emit('connection', ws, request);
    });
  } else {
    socket.destroy();
  }
});

// Start Server
const PORT = process.env.PORT || 5555;
server.listen(PORT, () => {
  console.log(`SnapDog Demo Backend listening on port ${PORT}`);
});
