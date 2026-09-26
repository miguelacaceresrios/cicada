//! Efectos: filtros por pista, delay y reverb comunes, y el limitador del
//! master. Todo es DSP propio en `f32` y sin estado compartido, para que dos
//! renders del mismo código den exactamente las mismas muestras.

use std::f32::consts::{PI, TAU};

pub type Frame = [f32; 2];

/// Umbral por debajo del cual una muestra se pone a cero.
///
/// Las colas de la reverb y del delay tienden a cero durante los silencios y
/// entran en el rango subnormal, donde cada operación cuesta decenas de
/// ciclos: un pasaje callado salía más caro de calcular que un drop.
const DENORMAL: f32 = 1e-20;

#[inline]
fn flush(x: f32) -> f32 {
    if x.abs() < DENORMAL { 0.0 } else { x }
}

pub const CHANNELS: usize = 2;

pub fn mix(a: Frame, b: Frame, amount: f32) -> Frame {
    [a[0] + b[0] * amount, a[1] + b[1] * amount]
}

/// Reparto estéreo de potencia constante: al mover una voz de un lado a otro
/// el volumen percibido no cambia.
pub fn pan(sample: f32, pan: f32) -> Frame {
    let angle = (pan.clamp(-1.0, 1.0) + 1.0) * PI / 4.0;
    [sample * angle.cos(), sample * angle.sin()]
}

/// Avanza un índice circular. El resto entero cuesta una división, y aquí se
/// hacen dos docenas por muestra entre la reverb y el delay.
#[inline]
fn wrap(index: usize, len: usize) -> usize {
    let next = index + 1;
    if next == len { 0 } else { next }
}

/// Filtro de dos polos con transformada que preserva la topología (Zavalishin).
/// Es estable a cualquier frecuencia de corte y no se descontrola al saturar.
#[derive(Clone, Copy, Default)]
pub struct Svf {
    ic1: f32,
    ic2: f32,
}

#[derive(Clone, Copy)]
pub struct SvfCoeffs {
    a1: f32,
    a2: f32,
    a3: f32,
    k: f32,
}

impl SvfCoeffs {
    pub fn new(cutoff: f32, q: f32, sr: f32) -> SvfCoeffs {
        let g = (PI * cutoff.clamp(20.0, sr * 0.45) / sr).tan();
        let k = 1.0 / q;
        let a1 = 1.0 / (1.0 + g * (g + k));
        let a2 = g * a1;
        SvfCoeffs {
            a1,
            a2,
            a3: g * a2,
            k,
        }
    }
}

/// Salidas del filtro: paso bajo y paso alto del mismo paso.
impl Svf {
    pub fn run(&mut self, x: f32, c: SvfCoeffs) -> (f32, f32) {
        let v3 = x - self.ic2;
        let v1 = c.a1 * self.ic1 + c.a2 * v3;
        let v2 = self.ic2 + c.a2 * self.ic1 + c.a3 * v3;
        self.ic1 = flush(2.0 * v1 - self.ic1);
        self.ic2 = flush(2.0 * v2 - self.ic2);
        (v2, x - c.k * v1 - v2)
    }

    pub fn low(&mut self, x: f32, c: SvfCoeffs) -> f32 {
        self.run(x, c).0
    }

    pub fn high(&mut self, x: f32, c: SvfCoeffs) -> f32 {
        self.run(x, c).1
    }
}

/// Filtro de una pista: paso bajo y paso alto opcionales, en estéreo.
#[derive(Clone, Copy, Default)]
pub struct TrackFilter {
    low: [Svf; CHANNELS],
    high: [Svf; CHANNELS],
}

impl TrackFilter {
    pub fn run(&mut self, frame: Frame, lpf: Option<SvfCoeffs>, hpf: Option<SvfCoeffs>) -> Frame {
        let mut out = frame;
        for (ch, sample) in out.iter_mut().enumerate() {
            if let Some(c) = lpf {
                *sample = self.low[ch].low(*sample, c);
            }
            if let Some(c) = hpf {
                *sample = self.high[ch].high(*sample, c);
            }
        }
        out
    }
}

/// Delay ping-pong sincronizado al tempo: cada repetición cruza de lado, que
/// es lo que da anchura sin emborronar el centro de la mezcla.
pub struct Delay {
    buffer: Vec<Frame>,
    index: usize,
    step: usize,
    feedback: f32,
    damp: [Svf; CHANNELS],
    damp_coeffs: SvfCoeffs,
}

impl Delay {
    pub fn new(seconds: f32, feedback: f32, sr: f32) -> Delay {
        let step = ((seconds * sr) as usize).max(1);
        Delay {
            buffer: vec![[0.0; CHANNELS]; step],
            index: 0,
            step,
            feedback,
            damp: Default::default(),
            // Cada repetición pierde agudos, como un eco real.
            damp_coeffs: SvfCoeffs::new(4_000.0, 0.707, sr),
        }
    }

    pub fn run(&mut self, input: Frame) -> Frame {
        let out = self.buffer[self.index];
        let damped = [
            self.damp[0].low(out[0], self.damp_coeffs),
            self.damp[1].low(out[1], self.damp_coeffs),
        ];
        // El canal cruzado es lo que hace el ping-pong.
        self.buffer[self.index] = [
            flush(input[0] + damped[1] * self.feedback),
            flush(input[1] + damped[0] * self.feedback),
        ];
        self.index = wrap(self.index, self.step);
        out
    }
}

/// Reverb tipo Freeverb: ocho filtros de peine en paralelo y cuatro
/// paso-todo en serie por canal. Los tamaños son los clásicos, desplazados
/// entre canales para abrir el estéreo.
pub struct Reverb {
    combs: [[Comb; 8]; CHANNELS],
    allpass: [[Allpass; 4]; CHANNELS],
}

const COMB_LEN: [usize; 8] = [1116, 1188, 1277, 1356, 1422, 1491, 1557, 1617];
const ALLPASS_LEN: [usize; 4] = [556, 441, 341, 225];
const STEREO_SPREAD: usize = 23;
const ROOM: f32 = 0.86;
const DAMP: f32 = 0.34;

impl Reverb {
    pub fn new(sr: f32) -> Reverb {
        // Los tamaños originales son para 44,1 kHz: se escalan al resto.
        let scale = |n: usize| ((n as f32 * sr / 44_100.0) as usize).max(1);
        let build = |ch: usize| {
            let offset = ch * STEREO_SPREAD;
            (
                std::array::from_fn(|i| Comb::new(scale(COMB_LEN[i] + offset))),
                std::array::from_fn(|i| Allpass::new(scale(ALLPASS_LEN[i] + offset))),
            )
        };
        let (c0, a0) = build(0);
        let (c1, a1) = build(1);
        Reverb {
            combs: [c0, c1],
            allpass: [a0, a1],
        }
    }

    pub fn run(&mut self, input: Frame) -> Frame {
        std::array::from_fn(|ch| {
            let x = (input[0] + input[1]) * 0.015;
            let mut wet: f32 = self.combs[ch].iter_mut().map(|c| c.run(x)).sum();
            for ap in &mut self.allpass[ch] {
                wet = ap.run(wet);
            }
            wet
        })
    }
}

struct Comb {
    buffer: Vec<f32>,
    index: usize,
    store: f32,
}

impl Comb {
    fn new(len: usize) -> Comb {
        Comb {
            buffer: vec![0.0; len],
            index: 0,
            store: 0.0,
        }
    }

    fn run(&mut self, x: f32) -> f32 {
        let out = self.buffer[self.index];
        self.store = flush(out * (1.0 - DAMP) + self.store * DAMP);
        self.buffer[self.index] = flush(x + self.store * ROOM);
        self.index = wrap(self.index, self.buffer.len());
        out
    }
}

struct Allpass {
    buffer: Vec<f32>,
    index: usize,
}

impl Allpass {
    fn new(len: usize) -> Allpass {
        Allpass {
            buffer: vec![0.0; len],
            index: 0,
        }
    }

    fn run(&mut self, x: f32) -> f32 {
        let buf = self.buffer[self.index];
        self.buffer[self.index] = flush(x + buf * 0.5);
        self.index = wrap(self.index, self.buffer.len());
        buf - x
    }
}

/// Taps por fase del detector de picos entre muestras.
const TP_TAPS: usize = 12;
/// Cuántas muestras intermedias se miran entre cada par de muestras reales.
const TP_PHASES: usize = 4;

/// Detector de pico real (inter-sample).
///
/// Mirar solo las muestras no basta: al reconstruir la onda, un conversor o
/// un codificador AAC pasan por valores más altos que cualquier muestra
/// suelta. Con la mezcla muy limitada esa diferencia llegaba a 3 dB, y ahí
/// es donde recorta de verdad. Se interpola a 4x solo para medir; el audio
/// que sale no pasa por este filtro.
struct TruePeak {
    coeffs: [[f32; TP_TAPS]; TP_PHASES],
    history: [[f32; TP_TAPS]; CHANNELS],
}

impl TruePeak {
    fn new() -> TruePeak {
        // Sinc enventanado con Hann, cortado a la Nyquist original.
        let center = (TP_TAPS * TP_PHASES) as f32 / 2.0 - 0.5;
        let mut coeffs = [[0.0; TP_TAPS]; TP_PHASES];
        for (phase, row) in coeffs.iter_mut().enumerate() {
            let mut sum = 0.0;
            for (tap, value) in row.iter_mut().enumerate() {
                let n = (tap * TP_PHASES + phase) as f32;
                let x = (n - center) / TP_PHASES as f32;
                let sinc = if x.abs() < 1e-6 {
                    1.0
                } else {
                    (PI * x).sin() / (PI * x)
                };
                let window = 0.5 - 0.5 * (TAU * n / (TP_TAPS * TP_PHASES - 1) as f32).cos();
                *value = sinc * window;
                sum += *value;
            }
            // Normalizar cada fase para que no altere el nivel de continua.
            for value in row.iter_mut() {
                *value /= sum;
            }
        }
        TruePeak {
            coeffs,
            history: [[0.0; TP_TAPS]; CHANNELS],
        }
    }

    /// El mayor valor absoluto que alcanza la onda alrededor de esta muestra.
    fn peak(&mut self, input: Frame) -> f32 {
        let mut worst = 0.0f32;
        for (ch, history) in self.history.iter_mut().enumerate() {
            history.rotate_right(1);
            history[0] = input[ch];
            worst = worst.max(input[ch].abs());
            for row in &self.coeffs {
                let mut sum = 0.0;
                for (c, h) in row.iter().zip(history.iter()) {
                    sum += c * h;
                }
                worst = worst.max(sum.abs());
            }
        }
        worst
    }
}

/// Limitador del master con escucha previa: retrasa la señal lo justo para
/// bajar el volumen *antes* de que llegue el pico, así el sonido se mantiene
/// fuerte sin el chasquido de un recorte duro.
/// Mínimo deslizante sobre una ventana.
///
/// Sin esto, un pico de una sola muestra apenas movía la ganancia: el
/// suavizado volvía a subir antes de que el pico llegara a la salida. Con el
/// mínimo, la ganancia empieza a bajar en cuanto el pico entra en la ventana
/// y llega a su sitio justo a tiempo.
struct SlidingMin {
    /// Valores de la ventana, en orden de llegada.
    values: Vec<f32>,
    write: usize,
    len: usize,
    /// Índices cuyos valores forman el mínimo creciente. El frente es el
    /// mínimo actual.
    queue: std::collections::VecDeque<usize>,
    /// Contador absoluto de muestras, para saber qué índices han caducado.
    now: usize,
}

impl SlidingMin {
    fn new(len: usize) -> SlidingMin {
        SlidingMin {
            values: vec![1.0; len],
            write: 0,
            len,
            queue: std::collections::VecDeque::with_capacity(len),
            now: 0,
        }
    }

    fn push(&mut self, value: f32) -> f32 {
        // Fuera los que ya no caben en la ventana.
        while self
            .queue
            .front()
            .is_some_and(|&i| i + self.len <= self.now)
        {
            self.queue.pop_front();
        }
        // Fuera los que este valor deja obsoletos: nunca volverán a ser mínimo.
        while self
            .queue
            .back()
            .is_some_and(|&i| self.values[i % self.len] >= value)
        {
            self.queue.pop_back();
        }
        self.values[self.write] = value;
        self.queue.push_back(self.now);
        self.write = wrap(self.write, self.len);
        self.now += 1;
        self.values[self.queue[0] % self.len]
    }
}

pub struct Limiter {
    lookahead: Vec<Frame>,
    index: usize,
    gain: f32,
    attack: f32,
    release: f32,
    threshold: f32,
    true_peak: TruePeak,
    required: SlidingMin,
}

impl Limiter {
    pub fn new(sr: f32) -> Limiter {
        let lookahead = ((0.004 * sr) as usize).max(1);
        Limiter {
            lookahead: vec![[0.0; CHANNELS]; lookahead],
            index: 0,
            gain: 1.0,
            // El ataque tarda una cuarta parte de la escucha previa, así que
            // llega a su valor antes de que el pico salga.
            attack: 1.0 - (-4.0 / (0.004 * sr)).exp(),
            release: 1.0 - (-1.0 / (0.15 * sr)).exp(),
            // Ajustado para que el pico real medido quede por debajo de
            // -1 dBTP, el techo que piden las plataformas para que la
            // codificación con pérdida no recorte. El detector propio es algo
            // menos fino que el de la norma, y este margen cubre la
            // diferencia.
            threshold: 0.81,
            true_peak: TruePeak::new(),
            required: SlidingMin::new(lookahead),
        }
    }

    pub fn run(&mut self, input: Frame) -> Frame {
        let delayed = self.lookahead[self.index];
        self.lookahead[self.index] = input;
        self.index = wrap(self.index, self.lookahead.len());

        let peak = self.true_peak.peak(input);
        let instant = if peak > self.threshold {
            self.threshold / peak
        } else {
            1.0
        };
        let target = self.required.push(instant);
        let coeff = if target < self.gain {
            self.attack
        } else {
            self.release
        };
        self.gain += (target - self.gain) * coeff;

        [
            safety(delayed[0] * self.gain),
            safety(delayed[1] * self.gain),
        ]
    }

    /// Muestras de retraso que introduce, para no cortar la cola al renderizar.
    pub fn latency(&self) -> usize {
        self.lookahead.len()
    }
}

/// Red de seguridad para los picos que el limitador no llega a atrapar.
///
/// Por debajo del codo no toca nada: aplicar un `tanh` a toda la señal, como
/// hacía la versión anterior, bajaba el volumen y ensuciaba la mezcla entera
/// para resolver un problema que solo aparece en unas pocas muestras.
fn safety(x: f32) -> f32 {
    const KNEE: f32 = 0.9;
    const HEAD: f32 = 1.0 - KNEE;
    let over = x.abs() - KNEE;
    if over <= 0.0 {
        return x;
    }
    (KNEE + HEAD * (over / HEAD).tanh()) * x.signum()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safety_only_touches_the_peaks() {
        for x in [-0.9, -0.5, 0.0, 0.25, 0.9] {
            assert_eq!(safety(x), x, "por debajo del codo no cambia nada");
        }
        assert!(safety(1.0) < 1.0 && safety(1.0) > 0.9);
        assert!(safety(50.0) <= 1.0);
        assert_eq!(safety(-2.0), -safety(2.0));
    }

    #[test]
    fn panning_keeps_power_constant() {
        for p in [-1.0, -0.5, 0.0, 0.5, 1.0] {
            let [l, r] = pan(1.0, p);
            assert!((l * l + r * r - 1.0).abs() < 1e-5, "pan {p}");
        }
        assert!(pan(1.0, -1.0)[1].abs() < 1e-6);
        assert!(pan(1.0, 1.0)[0].abs() < 1e-6);
    }

    #[test]
    fn lowpass_removes_highs_and_keeps_lows() {
        let sr = 44_100.0;
        let c = SvfCoeffs::new(500.0, 0.707, sr);
        let level = |freq: f32| {
            let mut svf = Svf::default();
            let n = 20_000;
            let out: f32 = (0..n)
                .map(|i| {
                    let x = (std::f32::consts::TAU * freq * i as f32 / sr).sin();
                    svf.low(x, c)
                })
                .skip(n / 2)
                .map(|v| v * v)
                .sum();
            (out / (n / 2) as f32).sqrt()
        };
        assert!(level(100.0) > 0.6, "los graves pasan");
        assert!(level(8_000.0) < 0.05, "los agudos se van");
    }

    #[test]
    fn limiter_holds_the_ceiling() {
        let sr = 44_100.0;
        let mut lim = Limiter::new(sr);
        let mut worst: f32 = 0.0;
        for i in 0..(sr as usize) {
            // Señal muy por encima del techo, con un golpe brusco en medio.
            let x = if i > sr as usize / 2 { 4.0 } else { 0.2 };
            let out = lim.run([x, -x]);
            if i > 100 {
                worst = worst.max(out[0].abs());
            }
        }
        assert!(worst <= 1.0, "nunca supera el máximo: {worst}");
        assert!(worst > 0.5, "pero deja pasar la música: {worst}");
    }

    #[test]
    fn delay_repeats_and_fades() {
        let sr = 1_000.0;
        let mut delay = Delay::new(0.1, 0.5, sr);
        let mut out = Vec::new();
        for i in 0..400 {
            let x = if i == 0 { 1.0 } else { 0.0 };
            out.push(delay.run([x, 0.0]));
        }
        // La primera repetición cruza al canal derecho.
        assert!(out[100][0].abs() > 0.5);
        assert!(out[200][1].abs() > 0.1);
        assert!(out[200][1].abs() < out[100][0].abs());
    }
}
