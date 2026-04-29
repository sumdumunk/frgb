pub mod common;

// --- Existing effects ---
pub mod breathing;
pub mod color_cycle;
pub mod double_meteor;
pub mod heartbeat;
pub mod meteor;
pub mod mixing;
pub mod rainbow;
pub mod rainbow_morph;
pub mod runway;
pub mod scan;
pub mod static_color;
pub mod taichi;
pub mod tide;
pub mod twinkle;
pub mod warning;

// --- Phase 1: Meteor variants ---
pub mod colorful_meteor;
pub mod meteor_contest;
pub mod meteor_mix;
pub mod meteor_rainbow;

// --- Phase 1: Arc/sweep ---
pub mod boomerang;
pub mod double_arc;
pub mod mop_up;
pub mod return_arc;
pub mod shuttle_run;

// --- Phase 1: Wave/flow ---
pub mod electric_current;
pub mod endless;
pub mod ripple;
pub mod river;

// --- Phase 1: Bounce/collision ---
pub mod collide;
pub mod ping_pong;
pub mod reflect;

// --- Phase 1: Fill/stack ---
pub mod door;
pub mod hourglass;
pub mod pioneer;
pub mod stack;

// --- Phase 1: Pulse/strobe ---
pub mod disco;
pub mod drumming;
pub mod heartbeat_runway;

// --- Phase 1: H2 (AIO) ---
pub mod bounce_effect;
pub mod pump;

// --- Phase 1: Segment/multi ---
pub mod candy_box;
pub mod duel;
pub mod gradient_ribbon;
pub mod lottery;
pub mod render;
pub mod staggered;
pub mod wing;

// --- Re-exports ---
pub use breathing::BreathingEffect;
pub use color_cycle::ColorCycleEffect;
pub use double_meteor::DoubleMeteorEffect;
pub use heartbeat::HeartBeatEffect;
pub use meteor::MeteorEffect;
pub use mixing::MixingEffect;
pub use rainbow::RainbowEffect;
pub use rainbow_morph::RainbowMorphEffect;
pub use runway::RunwayEffect;
pub use scan::ScanEffect;
pub use static_color::StaticColorEffect;
pub use taichi::TaichiEffect;
pub use tide::TideEffect;
pub use twinkle::TwinkleEffect;
pub use warning::WarningEffect;

pub use boomerang::BoomerangEffect;
pub use bounce_effect::BounceEffect;
pub use candy_box::CandyBoxEffect;
pub use collide::CollideEffect;
pub use colorful_meteor::ColorfulMeteorEffect;
pub use disco::DiscoEffect;
pub use door::DoorEffect;
pub use double_arc::DoubleArcEffect;
pub use drumming::DrummingEffect;
pub use duel::DuelEffect;
pub use electric_current::ElectricCurrentEffect;
pub use endless::EndlessEffect;
pub use gradient_ribbon::GradientRibbonEffect;
pub use heartbeat_runway::HeartBeatRunwayEffect;
pub use hourglass::HourglassEffect;
pub use lottery::LotteryEffect;
pub use meteor_contest::MeteorContestEffect;
pub use meteor_mix::MeteorMixEffect;
pub use meteor_rainbow::MeteorRainbowEffect;
pub use mop_up::MopUpEffect;
pub use ping_pong::PingPongEffect;
pub use pioneer::PioneerEffect;
pub use pump::PumpEffect;
pub use reflect::ReflectEffect;
pub use render::RenderEffect;
pub use return_arc::ReturnArcEffect;
pub use ripple::RippleEffect;
pub use river::RiverEffect;
pub use shuttle_run::ShuttleRunEffect;
pub use stack::StackEffect;
pub use staggered::StaggeredEffect;
pub use wing::WingEffect;

// --- Phase 2: UI-wired effects ---
pub mod blow_up;
pub mod cover_cycle;
pub mod meteor_shower;
pub mod paint;
pub mod snooker;
pub mod wave;

// --- Phase 2: TL-family effects ---
pub mod intertwine;
pub mod kaleidoscope;
pub mod racing;
pub mod tail_chasing;

pub use blow_up::BlowUpEffect;
pub use cover_cycle::CoverCycleEffect;
pub use intertwine::IntertwineEffect;
pub use kaleidoscope::KaleidoscopeEffect;
pub use meteor_shower::MeteorShowerEffect;
pub use paint::PaintEffect;
pub use racing::RacingEffect;
pub use snooker::SnookerEffect;
pub use tail_chasing::TailChasingEffect;
pub use wave::WaveEffect;
