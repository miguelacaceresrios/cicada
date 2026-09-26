# Arquitectura

## Pipeline

```text
texto ─► lexer ─► parser ─► AST ─► sema ─► Program ─► query(rango) ─► eventos (IR) ─► audio / visuales
```

| Etapa | Módulo | Entrada → salida | Detecta |
| --- | --- | --- | --- |
| Lexer | `core/lexer.rs` | `&str` → líneas de tokens con posición | caracteres inválidos, comillas sin cerrar |
| Parser | `core/parser.rs` | tokens → AST | sintaxis, grupos sin cerrar, `*` mal aplicado |
| Análisis | `core/sema.rs` | AST → `Program` | pistas sin sonido, golpes en un synth, rangos |
| Patrones | `core/pattern.rs`, `core/program.rs` | `Program` + rango → `Vec<Event>` | nada: son funciones puras |
| Audio | `audio/engine.rs`, `audio/voice.rs` | eventos → muestras | samples ilegibles, dispositivo |

Todos los errores anteriores al audio llevan línea y columna, porque
`cicada check` es la primera interfaz del usuario.

## Patrones como consultas

Cada patrón se aplana una sola vez en *slots*: desplazamiento y duración
dentro de un ciclo, como fracciones exactas. Después, la pieza se consulta
por rangos:

```rust
program.query(from, to) // → eventos que empiezan en [from, to)
```

Consultar un rango da exactamente lo mismo que consultar sus partes y
concatenarlas (hay un test que lo comprueba). Por eso el mismo código sirve
para tres casos:

- **Render offline**: se consulta toda la pieza de una vez.
- **Tiempo real**: en la 0.1 también se consulta entera antes de abrir el
  stream. En la 0.3 se consultarán ventanas cortas por delante del cursor.
- **Live coding (0.3)**: se sustituye el `Program` entre dos consultas y el
  audio no se corta.

La aleatoriedad (`?`) es un hash de `(seed, pista, ciclo, capa, paso)`, no un
generador secuencial, así que tampoco depende del orden de las consultas.

## CICADA IR

`cicada ir pieza.cicada` la imprime en JSON. Es el contrato con cualquier
motor de salida:

```json
{
  "bpm": 200.0, "beats": 4, "seed": 2026,
  "length_cycles": 52, "seconds_per_cycle": 1.2,
  "tracks":   [{ "name": "kick", "sound": "kick" }],
  "sections": [{ "name": "drop", "kind": "drop", "start": 16, "length": 16, "energy": 1.0 }],
  "events":   [{ "start": [33, 2], "duration": [1, 16], "track": 0, "section": 2,
                 "type": "hit", "velocity": 1.0, "gain": 1.0, "dist": 0.9,
                 "src": { "line": 22, "col": 9, "len": 1 } }]
}
```

Los tiempos van en ciclos como `[numerador, denominador]`. Para pasarlos a
segundos se multiplican por `seconds_per_cycle`.

## Audio

```text
                        ┌── lpf/hpf ──┬─────────────► master ─► limitador ─► estéreo
eventos ─► voces ─pan──►┤ (por pista)  ├── envío ─► delay ──┤                   ├── WAV
                        └──────────────┴── envío ─► reverb ─┘                   └── Player (CPAL)
```

Los filtros y los envíos van en el bus de cada pista, no en cada voz: filtrar
golpe a golpe no tendría sentido musical, y una cola de delay por voz se
cortaría al apagarse la nota. Una pista sin efectos salta el bus y va directa
al master.

- La salida es **estéreo intercalado** (izquierda, derecha, izquierda…). Las
  voces son mono y se reparten con potencia constante según su `pan`.
- La pieza se renderiza entera antes de sonar: un minuto de música tarda menos
  de un segundo. El `Player` solo copia muestras al dispositivo, así que lo
  que suena es exactamente lo que se exporta. Además, pausar, saltar a otro
  momento y recargar en caliente son operaciones triviales.
- El `Player` acepta salidas en `f32`, `i16`, `u16` e `i32`. Nunca bloquea el
  hilo de audio: si la interfaz tiene el estado tomado, ese bloque suena en
  silencio.
- Cada evento arranca en su **muestra exacta** (hay un test para ello), no al
  principio del bloque.
- El ruido de snare y hat es un xorshift sembrado por evento, así que es
  determinista. La reverb, el delay y el limitador tampoco guardan estado
  entre renders.

### El master

El master no recorta: lleva un **limitador con escucha previa** de 4 ms, que
baja el volumen *antes* de que llegue el pico. Así la mezcla puede ir tan alta
como pide el género sin el chasquido de un recorte duro. Detrás queda un
recorte suave que solo actúa por encima de 0.9, como red de seguridad.

Esos 4 ms de escucha previa retrasarían todo el audio, así que `render`
descarta las primeras muestras: cada evento vuelve a caer en su muestra exacta
y el vídeo sigue cuadrando al frame (hay un test que lo comprueba).

El WAV se escribe con ruido de dispersión triangular al pasar a 16 bits, que
reparte el error de redondeo en vez de dejarlo pegado a la señal; sin él las
colas largas de reverb se apagan con un zumbido áspero.

La cola del render dura lo que suena: se recorta el silencio del final, pero
nunca por debajo de lo que declara `length`.

Los sonidos integrados están nivelados entre sí para que `gain 1` signifique
lo mismo en todos y el kick quede como el elemento más fuerte.

## Visuales

```text
Program + audio ─► Timeline ─► FrameState(t) ─► shader (wgpu) ─► overlay (CPU) ─► RGBA ─► FFmpeg ─► MP4
```

| Pieza | Módulo | Qué hace |
| --- | --- | --- |
| Línea de tiempo | `visual/timeline.rs` | Traduce la IR a señales continuas para cualquier `t`: impulsos por rol, energía suavizada, escena y fundido, destellos, nivel del audio, pasos activos. Es pura y se testea sin GPU. |
| GPU | `visual/gpu.rs`, `visual/shader.wgsl` | Un shader a pantalla completa, renderizado sin ventana y copiado a memoria. |
| Overlay | `visual/overlay.rs` | El código con resaltado de sintaxis y los pasos que suenan iluminados, el HUD y la cabecera de título. Se compone en CPU con `fontdue`. |
| Vídeo | `visual/video.rs` | Manda los frames por una tubería a FFmpeg (H.264 + AAC). |

Cada frame `f` se calcula en `t = f / fps` directamente a partir de la IR, así
que la sincronía es exacta por construcción y no hay estado acumulado. Las
visuales combinan tres capas:

| Capa | Fuente | Uso |
| --- | --- | --- |
| Impulsos | golpes de cada rol (kick, snare, hat, notas) | zoom y aberración con el kick, glitch con el snare, chispas con el hat |
| Energía | `energy` de la sección y nivel RMS del audio (media de 0,3 s) | velocidad, intensidad y brillo general |
| Estructura | `kind` de la sección | escena propia con fundido de 0,4 s: túnel (intro), rejilla (build), rayos (drop), niebla (break) |

El rol de una pista sale de su sonido (`kick`, `snare`, `hat`, `synth`). Para
los samples se deduce del nombre de la pista.

Cada evento lleva la posición en el código del paso que lo generó (`src` en
la IR). Así el overlay ilumina exactamente el carácter que está sonando. El
editor de la 0.4 usará la misma información.

### Fotosensibilidad

A 200 bpm, un pulso de luz en cada kick sería un estrobo de más de tres
destellos por segundo. Por eso:

- el kick se expresa en **movimiento** (zoom, geometría), no en brillo;
- el nivel del audio que modula el brillo se promedia en 0,3 s;
- los destellos explícitos (inicio de cada drop, kicks acentuados en secciones
  intensas) pasan por un limitador: nunca menos de 1/3 s entre dos. Hay un test
  que lo comprueba.

Midiendo la luminancia media de cada frame de `examples/hakken.mp4`, el peor
segundo tiene 2 transiciones grandes, por debajo del umbral de 3 de WCAG 2.3.1.
Es una aproximación: no sustituye a un analizador como PEAT y no evalúa la
regla específica de destellos rojos saturados.

## Repositorio

```text
cicada/
├── crates/
│   ├── cicada-core/    # lenguaje, patrones e IR; solo depende de serde
│   ├── cicada-audio/   # voces, motor, WAV (hound), tiempo real (cpal, opcional)
│   ├── cicada-visual/  # línea de tiempo, shader wgpu, overlay y vídeo
│   ├── cicada-cli/     # check · render · play · video · frame · ir
│   └── cicada-studio/  # app de escritorio (egui): editor, vista previa, exportación
├── examples/           # piezas de ejemplo; también se usan en los tests
└── docs/
```

`cicada-core` no depende de audio, de E/S ni de ninguna UI. Así es fácil de
testear y se podrá compilar a WASM para un playground web.

## CICADA Studio

Es una app nativa en egui, en lugar de Tauri + React: así la vista previa usa
el mismo shader `wgpu` que el vídeo exportado, sin reescribir las visuales en
JavaScript, y todo cabe en un solo `.exe`.

```text
tecla ─► espera 250 ms ─► compile ─► errores en el editor
                              └─► render en otro hilo ─► Player.set_buffer (misma posición)
                                                        └─► Visualizer.load (misma GPU)
cada frame: t = Player.position() ─► FrameState ─► vista previa + pasos iluminados en el editor
```

Si el código nuevo tiene errores, sigue sonando la última versión válida.
`CICADA_STUDIO_SCREENSHOT="captura.png@47"` abre la app, salta al segundo 47,
guarda una captura y cierra; así se generó `docs/studio.png`.

## Tests

| Qué | Dónde |
| --- | --- |
| Fracciones, notas, lexer y aplanado de patrones | tests unitarios en cada módulo |
| Lenguaje completo y errores con su posición | `core/tests/language.rs` |
| Consulta por partes = consulta entera | `query_is_the_union_of_its_parts` |
| Mismo código ⇒ mismas muestras | `audio/tests/render.rs` |
| La salida es estéreo y el `pan` la abre | `output_is_stereo` |
| El master nunca recorta, ni con todo a tope | `master_never_clips` |
| Filtros, delay y reverb cambian el sonido | `filters_and_sends_change_the_sound` |
| Eventos en su muestra exacta | `events_land_on_the_exact_sample` |
| Nunca más de 3 destellos por segundo | `flashes_never_exceed_three_per_second` |
| Impulsos, fundidos y pasos activos | `visual/timeline.rs` |

El determinismo está garantizado en una misma plataforma. Entre sistemas
operativos, `sin`, `exp` y `tanh` de la librería estándar pueden diferir en
el último bit. Si hacen falta renders idénticos bit a bit entre máquinas, el
siguiente paso es usar implementaciones propias de esas funciones.
