# Lenguaje CICADA 0.1

Este documento describe lo que el parser acepta hoy. Todo lo que no aparece
aquí queda fuera de la 0.1 a propósito.

```text
# Los comentarios empiezan con #
bpm 200
seed 2026

bass = synth saw gain 0.7          # declarar un sonido
hat  x?x?x?x? gain 0.5             # fuera de secciones: suena en toda la pieza

intro 8:                           # sección de 8 ciclos
  energy 0.3
  kick x...x...x...x... dist 0.4

drop 16:
  energy 1
  kick X...x...X...x... dist 0.9
  bass C1 C1 G0 [C1 C2]
```

## Tiempo

- La unidad es el **ciclo**. Por defecto un ciclo es un compás de 4 beats
  (se cambia con `beats N`).
- Cada patrón **reparte sus pasos a lo largo de un ciclo**. `x.x.x.x.` tiene
  8 pasos de 1/8 de ciclo; `xxxx` tiene 4 pasos de 1/4.
- Internamente el tiempo es una **fracción exacta**, nunca un `f64`. Solo se
  convierte a muestras en el motor de audio.

## Tipos de línea

| Forma | Ejemplo | Qué es |
| --- | --- | --- |
| `ajuste valor` | `bpm 195` | Ajuste global |
| `pista = sonido [mods]` | `bass = synth saw gain 0.8` | Declaración de sonido |
| `nombre ciclos:` | `drop 16:` | Sección; su cuerpo va indentado |
| `pista patrón [mods]` | `kick x.x.x.x. dist 0.7` | Patrón |
| `energy valor` | `energy 0.8` | Solo dentro de una sección |

El orden de las declaraciones no importa: se puede usar `bass` antes de
declararlo.

### Ajustes

| Ajuste | Valor | Por defecto |
| --- | --- | --- |
| `bpm` | mayor que 0, como mucho 1000 | 120 |
| `beats` | beats por ciclo, entero de 1 a 32 | 4 |
| `seed` | entero ≥ 0 | 0 |
| `length` | duración en ciclos si no hay secciones | 8 |

### Sonidos

| Sonido | Tipo de patrón | Descripción |
| --- | --- | --- |
| `kick` | golpes | Seno con barrido de pitch; con `dist` se vuelve el kick gabber |
| `snare` | golpes | Tono corto con ruido filtrado |
| `hat` | golpes | Ruido agudo con envolvente corta |
| `synth saw` / `square` / `sine` | notas | Oscilador sin aliasing (PolyBLEP) con envolvente |
| `sample "ruta.wav"` | golpes | WAV relativo al archivo `.cicada` |

`kick`, `snare` y `hat` se pueden usar sin declararlos. Cualquier otro nombre
necesita su declaración.

### Secciones

Las secciones se suceden en el orden en que se escriben, sin huecos. Los
patrones escritos fuera de cualquier sección suenan en todas. Si no hay
secciones, la pieza dura `length` ciclos.

El **tipo** de la sección se deduce de su nombre: `intro`, `build`, `drop`,
`break` y `outro` (también `drop2`, `drop_b`…). El tipo y la `energy` (de 0
a 1, por defecto 0.5) viajan en la IR para que las visuales sepan en qué
momento de la pieza están.

### Modificadores

Van al final de una declaración de sonido (valor por defecto de la pista) o
de un patrón (solo para esa línea).

| Modificador | Rango | Por defecto | Efecto |
| --- | --- | --- | --- |
| `gain` | 0 a 4 | 1 | Volumen |
| `dist` | 0 a 1 | 0 | Saturación (`tanh`) |
| `prob` | 0 a 1 | 0.5 | Probabilidad de los golpes `?` |

## Patrones

| Símbolo | Significado |
| --- | --- |
| `x` | golpe (velocidad 0.8) |
| `X` | golpe acentuado (velocidad 1.0) |
| `?` | golpe con probabilidad `prob` |
| `.` o `~` | silencio |
| `C1`, `F#2`, `Bb0` | nota (`C4` = 60 en MIDI) |
| `[ … ]` | grupo: ocupa un paso y se subdivide por dentro |
| `a*n` | repite `a` n veces dentro de su paso |

En los golpes cada carácter es un paso, así que `x.x.` son cuatro pasos. Las
notas se separan con espacios. Un patrón no puede mezclar golpes y notas.

```text
kick  x x x [x x]      # el último paso se divide en dos
hat   x*16             # 16 golpes por ciclo
bass  C1 ~ [C1 C2] G0
kick  [x.]*4           # * sobre varios pasos necesita un grupo
```

## Aleatoriedad determinista

Cada `?` se decide con un hash, no con un generador secuencial:

```text
hash(seed, pista, ciclo, capa, paso) → [0, 1)
```

Por eso empezar a reproducir desde el ciclo 40, renderizar offline o recargar
en vivo dan exactamente el mismo resultado.

## Errores

`cicada check` señala el problema con línea, columna y una ayuda:

```text
error: `z` no es válido en un patrón
 --> track.cicada:3:8
  |
3 | kick x.z.
  |        ^
  = ayuda: golpes: x X ? .  ·  notas: C1 F#2 Bb0  ·  silencio: ~
```

## Pendiente

- Transformaciones: `rev`, `fast`, `slow`, `every`, `euclid(3,8)`
- Automatización de parámetros dentro de una sección
- Cadenas de FX por pista (filtros, delay, reverb, bitcrush)
- Bloque `visual` para asociar eventos a efectos (ver el roadmap)
