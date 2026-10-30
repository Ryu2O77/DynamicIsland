# Documentación de Dynamic Island

Isla dinámica para Windows escrita en Rust: una píldora negra anclada al borde superior de la pantalla que se expande al pasar el mouse y reúne música con letras sincronizadas, reloj y clima, calendario, temporizadores, calculadora, tareas, lector de EPUB, selector de color, estadísticas del sistema, capturas de pantalla y un mezclador de volumen.

> Proyecto hecho con IA para uso personal. Solo funciona en Windows 10/11.

---

## 1. Contenido

1. [Instalación y compilación](#2-instalación-y-compilación)
2. [Uso general](#3-uso-general)
3. [Guía de cada pestaña](#4-guía-de-cada-pestaña)
4. [Ajustes](#5-ajustes)
5. [Archivos de datos y configuración](#6-archivos-de-datos-y-configuración)
6. [Atajos y bandeja del sistema](#7-atajos-y-bandeja-del-sistema)
7. [Arquitectura](#8-arquitectura)
8. [Cómo extenderlo](#9-cómo-extenderlo)
9. [Limitaciones y solución de problemas](#10-limitaciones-y-solución-de-problemas)

---

## 2. Instalación y compilación

Requisitos: [Rust](https://rustup.rs) y las *Visual Studio C++ Build Tools*.

```powershell
git clone <url-del-repositorio>
cd DynamicIsland
cargo build --release
.\target\release\dynamic_island.exe
```

El ejecutable queda en `target\release\dynamic_island.exe`. Para cerrarlo, clic derecho sobre la isla o el icono de la bandeja y elegir **Salir**.

Si al recompilar aparece *"Acceso denegado"* al reemplazar el `.exe`, la isla sigue abierta: ciérrala desde la bandeja (o con `taskkill /IM dynamic_island.exe /F`) y vuelve a compilar.

Dependencias: solo la crate `windows` (0.58). No se usan dependencias de terceros para el ZIP, la descompresión, el análisis de XHTML, el PNG ni el clima.

Perfil *release*: `opt-level = "z"`, `lto`, `codegen-units = 1`, `panic = "abort"` y `strip`. Al usar `panic = "abort"`, el código no debe depender de capturar *panics*.

---

## 3. Uso general

- Pasa el mouse por la isla para **expandirla**; al sacarlo se colapsa.
- La **barra de pestañas** está centrada arriba. En el tema predeterminado solo muestra iconos y la campana (notificaciones) y el punto (*pin*) quedan espejados a cada lado.
- El **punto de la derecha** (*pin*) mantiene la isla abierta.
- **Clic derecho** en la isla o en el icono de la bandeja abre el menú.
- Con la isla cerrada se muestra lo más relevante en este orden: temporizador/pomodoro activo, cronómetro, canción (o letra), clima o reloj.
- **Mover la isla:** con *Mover libremente* activado se arrastra a cualquier parte. En estilo notch, arrastrada hacia los lados sigue pegada al borde superior; si se baja, se despega y se convierte en una isla flotante. Al soltarla cerca del borde vuelve a pegarse. *Restablecer* la centra.
- **Ocultar en pantalla completa:** la isla desaparece mientras haya un juego o vídeo a pantalla completa.
- **Esquivar pestañas del navegador:** manteniendo el mouse sobre el borde superior sobre la isla durante un segundo, se desliza hacia abajo y vuelve al alejar el cursor.
- **Opacidad por proximidad:** la isla se atenúa cuando el cursor está lejos.

---

## 4. Guía de cada pestaña

### Música
Controla lo que suena en cualquier aplicación que exponga controles multimedia (Spotify, navegadores, foobar2000…).

- Anterior / reproducir-pausar / siguiente y barra de progreso (clic para saltar).
- **Carátula** y **visualizador** (barras animadas), ambos opcionales en *Ajustes*. Si se oculta la carátula, el texto se reacomoda sin dejar un recuadro vacío.
- **Letras:** el botón de la derecha abre la letra sincronizada (se busca en línea). Con *Letra en la isla* la línea actual también se muestra con la isla cerrada.
- **Corazón:** envía a Spotify el atajo *Guardar en Tus me gusta* (`Alt+Shift+B`). Trae un instante la ventana de Spotify al frente, envía el atajo y devuelve el foco. Requiere la ventana de Spotify abierta y no minimizada; **no refleja** si la canción ya tenía "me gusta".
- **Chip de fuente** (izquierda): alterna entre *Auto* (sigue la sesión actual del sistema), cada aplicación con controles multimedia, y **Off** (no mostrar la canción en la isla cerrada).
- **Cambio de canción:** con la isla cerrada, la canción que termina sale deslizándose a la izquierda mientras la nueva entra desde la derecha.

### Reloj y Clima
- **Reloj:** hora, y junto a ella el clima y la batería cuando están disponibles.
- **Clima:** condiciones actuales y pronóstico de 3 días con [Open-Meteo](https://open-meteo.com) (sin clave de API). Se actualiza cada 15 minutos mientras se muestra. La ciudad, las unidades (°C/°F), la detección de ubicación por IP y la actualización manual están en *Ajustes → Clima*.

### Calendario
Flechas para cambiar de mes y botón *Hoy* para volver al mes actual.

### Tiempo
Pomodoro, temporizador y cronómetro en una sola pestaña.

- **Pomodoro:** 25 min de enfoque, 5 min de descanso corto y 15 min de descanso largo cada 4 rondas. Iniciar/pausar, reiniciar y saltar fase.
- **Timer:** atajos de 1 / 5 / 10 / 25 minutos, pausa, reinicio y +1 minuto. La cuenta atrás se ve también con la isla cerrada.
- **Cronómetro:** décimas de segundo y vueltas.

### Volumen (mezclador)
Volumen general y por aplicación, con botón de silencio (Core Audio).

- Muestra las aplicaciones de **todos los dispositivos de salida** (por ejemplo las que van a Voicemeeter), no solo las del dispositivo predeterminado. Las sesiones propias de Voicemeeter se ocultan.
- Clic en el **nombre** de una aplicación (o clic derecho sobre el nombre o el silencio) abre un menú para elegir su dispositivo de salida.
- Sin pestaña abierta, la **rueda del mouse** sobre la isla ajusta el volumen general (opción *Volumen con la rueda*).

### Calculadora
Teclas con el mouse o el teclado: dígitos, `+ - * /`, paréntesis, `Enter` para `=`, `Esc` o `C` para borrar todo y `Retroceso` para un dígito. Muestra el resultado en vivo.

### Tareas
Escribe y pulsa `Enter` para añadir. Clic en una tarea para marcarla como hecha, papelera para borrarla, rueda del mouse para desplazar la lista. Se guardan localmente.

### Libro (lector de EPUB)
- Clic en el **título** del libro (o en *Abrir…* si no hay ninguno) para elegir un `.epub`.
- Texto paginado a la medida de la isla: clic en la parte izquierda del texto (o `‹`) = página anterior; en el resto (o `›`) = siguiente. La rueda del mouse también pasa páginas.
- `«` y `»` cambian de capítulo. El texto del pie (`cap/total · título · pág.`) abre el **índice de capítulos**: rueda para desplazar y clic para saltar.
- **Aa** cambia entre tres tamaños de letra (se guarda).
- Muestra el porcentaje total de lectura y **recuerda** el libro y la posición exacta.
- Solo texto (sin imágenes ni estilos). No admite libros con DRM.

### Color
- **Cuentagotas:** sigue al cursor por toda la pantalla y muestra el color en vivo. Clic o `Enter` fija el color; `Esc` cancela. La isla se mantiene abierta durante la selección.
- Al fijar un color se copia su código hexadecimal al portapapeles y se guarda en el **historial** (últimos 12). Clic en un color del historial lo vuelve a copiar. *Copiar* copia el color actual.
- El clic con que se fija el color también llega a la aplicación que está debajo; si eso molesta, usa `Enter`.

### Sistema
Cuadrícula de 2×2 tarjetas estilo Apple: **CPU** y **Memoria** (anillo de progreso con gráfico del último minuto), **Disco C:** (anillo con espacio libre y total) y **Red** (velocidades de bajada y subida). Al pie, el tiempo encendido y la batería.

### Captura
Captura la pantalla completa (todos los monitores) y la guarda como PNG. El botón inferior abre la carpeta de capturas.

### Ajustes
Ver la sección siguiente.

---

## 5. Ajustes

La pestaña se organiza en **tres columnas de tarjetas**:

| Columna | Contenido |
| --- | --- |
| **Pestañas** | Activar o desactivar Música, Reloj, Calendario, Pomodoro y timer, Cronómetro, Calculadora, Tareas, Libro, Color, Clima, Sistema, Captura y Mezclador. |
| **Extras / Apariencia** | *Extras*: Letras, Letra en la isla, Notificaciones (botón de campana) y Aviso de batería. *Apariencia*: Tema, Estilo, Fuente, Acento, Sombra, Visualizador y Carátula. |
| **Comportamiento / Clima** | *Comportamiento*: Mover libremente, Posición (Restablecer), Opacidad por proximidad (opacidad mínima y distancia), Ocultar en fullscreen, Volumen con la rueda, Iniciar con Windows y Esquivar pestañas web. *Clima*: ciudad (con Buscar), unidades, detectar ubicación y actualizar. |

Los temas son **Predeterminado** (estilo Apple: negro puro, grises iOS, acento naranja, pestañas centradas y teclas en cápsula), **NieR claro** y **NieR oscuro** (misma disposición, con sus propios colores y decoraciones). El **acento** puede ser el del tema, naranja, azul, verde, rosa, morado o blanco. El **estilo** puede ser *Isla* (flotante) o *Notch* (pegada al borde).

Todas las funciones también se pueden activar desde el submenú *Funciones* del menú de la bandeja.

---

## 6. Archivos de datos y configuración

Todo vive en `%APPDATA%\DynamicIsland\`:

| Archivo | Contenido |
| --- | --- |
| `config.ini` | Funciones activadas (una línea `clave=1/0` por función), tema, estilo, fuente, acento, posición, opacidad, distancia, ciudad y coordenadas, unidades, estado de las letras, fuente multimedia y tamaño de letra del lector (`book_size`). |
| `todo.txt` | Tareas (`1\|texto` hecha, `0\|texto` pendiente). |
| `book.ini` | Último libro abierto y posición (`path`, `chapter`, `para`, `off`). |
| `colors.txt` | Historial de colores, un hex (`RRGGBB`) por línea, el más reciente primero. |

Las capturas se guardan en `%USERPROFILE%\Pictures\DynamicIsland\Captura_AAAA-MM-DD_HH-MM-SS.png`.

Borrar un archivo restablece su contenido a los valores por defecto. Las claves desconocidas en `config.ini` se ignoran.

---

## 7. Atajos y bandeja del sistema

| Atajo | Acción |
| --- | --- |
| `Ctrl+Alt+S` | Captura de pantalla (global). Si el atajo está ocupado, la isla avisa y se puede usar el menú de la bandeja. |
| `Alt+Shift+B` | Lo envía la isla a Spotify con el botón del corazón. |

**Menú de la bandeja / clic derecho:** Ajustes, submenú *Funciones*, *Restablecer posición*, *Captura de pantalla*, *Abrir capturas*, *Iniciar con Windows* y *Salir*.

---

## 8. Arquitectura

### 8.1 Módulos

| Archivo | Responsabilidad |
| --- | --- |
| `src/main.rs` | Ventana, bucle de mensajes, estado de la aplicación (`App`), diseño de widgets, dibujo, animación, configuración y lógica de cada pestaña. |
| `src/epub.rs` | Lector de EPUB sin dependencias: descompresión *inflate*, lectura del directorio central del ZIP, `container.xml` → OPF → *spine*, conversión de XHTML a párrafos y paginador. |
| `src/mixer.rs` | Mezclador con Core Audio: enumeración de dispositivos y sesiones de audio, volumen/silencio, cambio de salida por aplicación. |
| `src/system.rs` | Estadísticas: CPU, RAM, disco, red y tiempo encendido con APIs nativas. |
| `src/weather.rs` | Clima con WinHTTP (TLS 1.2/1.3) y Open-Meteo; búsqueda de ciudad y ubicación por IP. |
| `src/capture.rs` | Captura con GDI (`BitBlt`) y codificación PNG con WIC a través de WinRT. |

### 8.2 Ventana y renderizado

- **Una sola ventana Win32 en capas** (*layered*) con transparencia por píxel. No hay framework de interfaz.
- El **lienzo** mide 700 × 430 px lógicos (`CW`, `CH`); la isla se dibuja dentro y se centra. Anchos: `EW` = 440 (normal), `EW_WIDE` = 620 (Tiempo y Reloj con clima) y `EW_SET` = 660 (Ajustes).
- **Renderizado por software** a un búfer propio: rectángulos redondeados y círculos mediante campos de distancia con signo (SDF) con bordes suavizados; líneas, arcos, anillos y polígonos para iconos y gráficos.
- **Texto con GDI** (`DrawTextW`, `GetTextExtentPoint32W`) sobre el mismo búfer, en una pasada posterior.
- Una pasada final convierte a **alfa premultiplicado**, enmascarada por la cobertura de la píldora (`cov`), de modo que nada se dibuje fuera de ella (incluida la animación de cambio de canción).
- **Dibujo en dos pasadas:** `draw_software` (formas) y `draw_text` (texto). Ambas reciben la misma lista de widgets.

### 8.3 Estado y modelo de widgets

- `App` es el estado global, guardado en un `thread_local` (`APP`) y accedido con `with_app`.
- `layout()` construye cada fotograma una lista de `Widget` (rectángulo, tipo `WKind`, acción `Act`, etiqueta y valores). Esa misma lista se usa para:
  - **dibujar** (`draw_widgets`),
  - **detectar el mouse** (`hit`, hover y clics),
  - **dibujar las etiquetas** de texto.
- `Page` identifica la pestaña activa; `Mode` indica si la isla está colapsada, expandida, mostrando un aviso (*toast*), etc.
- `do_act(Act)` centraliza las acciones de la interfaz.
- `target()` devuelve el tamaño objetivo de la isla según el modo y la página.

### 8.4 Animación

- La animación es un **resorte** (k = 260, c = 26) integrado con paso fijo, sincronizada con el compositor del escritorio (DWM) y activa solo mientras algo se mueve; en reposo el uso de CPU es prácticamente nulo.
- Un hilo (`anim_thread`) despierta a la ventana con `WM_APP_FRAME` y `tick()` avanza el resorte y la opacidad.
- Los cambios de vista usan un fundido (`fade`, 0,25 s con *smoothstep*). El cambio de canción con la isla cerrada usa un deslizamiento lateral.
- Temporizadores de Windows: reloj (1 s), visualizador (80 ms), proximidad (100 ms) y el cuentagotas (25 ms mientras está activo).

### 8.5 Hilos y comunicación

| Hilo | Tarea |
| --- | --- |
| Principal | Ventana, mensajes, dibujo y lógica. |
| `media_thread` | Consulta las sesiones multimedia (WinRT) y avisa con `WM_APP_MEDIA`. |
| `anim_thread` | Marca los fotogramas de animación. |
| Clima / letras / libro | Trabajos puntuales (WinHTTP, carga de EPUB) que dejan el resultado en una variable compartida y avisan con un mensaje (`WM_APP_WEATHER`, `WM_APP_BOOK`…). |

### 8.6 Lector de EPUB (`epub.rs`)

1. `Zip` lee el directorio central y extrae entradas (con `inflate` propio, estilo *puff*).
2. `container.xml` indica el archivo OPF; del OPF se obtiene el *manifest* y el *spine*.
3. Cada XHTML del *spine* se convierte en párrafos de texto (`html_to_paras`); los títulos se detectan como encabezados.
4. `paginate` reparte un capítulo en páginas con ajuste de línea *greedy*, parametrizado por una función de medición (la medida real de GDI) y por las líneas por página, que dependen del tamaño de letra.
5. `page_for` convierte una posición (párrafo, desplazamiento en bytes) en el número de página, lo que permite conservar el lugar al cambiar el tamaño de letra.

El archivo no usa dependencias y tiene pruebas nativas independientes de Windows.

### 8.7 Persistencia

`load_config`/`save_config` (INI sencillo), `load_todos`/`save_todos`, `load_book_state`/`save_book_state` y `load_colors`/`save_colors`. Todas fallan en silencio y vuelven a los valores por defecto.

---

## 9. Cómo extenderlo

### Añadir una pestaña

1. **`Page`:** agrega la variante al enum.
2. **Función activable:** crea la constante `F_*` (los índices son consecutivos, `F_COUNT` es el total), añade la entrada `("clave", "Etiqueta")` a `FEATURES` en el mismo orden y, si debe verse en Ajustes, agrégala a la columna correspondiente en `layout()`.
3. **Barra de pestañas:** añade `(Page::X, "Nombre", ancho)` a `TABS` y ajusta el tamaño del arreglo.
4. **Icono:** agrega la rama en `tab_icon`.
5. **Visibilidad:** rama en `page_enabled`.
6. **Tamaño:** rama en `target_base` con `(ancho, alto)`.
7. **Widgets:** rama en `layout()` con los `Widget` que necesite (posiciones relativas a `cx` y `y0`, multiplicadas por la escala `s`).
8. **Dibujo:** formas en `draw_software` y texto en `draw_text`.
9. **Acciones:** nuevas variantes en `Act` y su tratamiento en `do_act`.
10. **Documentación:** actualiza este archivo y el README.

Si el compilador se queja de un arreglo de tamaño fijo (`TABS`, `FEATURES`, `[bool; F_COUNT]`), ajusta el número.

### Añadir un tipo de widget
Agrega la variante a `WKind`, su dibujo en `draw_widgets` (o en `draw_software` si necesita datos propios) y, si lleva texto, su caso en el bucle de etiquetas de `draw_text`.

### Añadir un tema
Añade una entrada a `THEMES` y su nombre a `THEME_NAMES`; el selector de Ajustes la incluye automáticamente.

### Convenciones
- Las posiciones se escriben en píxeles lógicos multiplicados por `s` (escala de DPI).
- Nada debe provocar *panics* (`panic = "abort"`): usa `get`, `checked_*` y manejo de errores.
- Las operaciones lentas (red, disco, descompresión) van en hilos y devuelven el resultado por mensaje.

---

## 10. Limitaciones y solución de problemas

- Solo Windows 10/11. La interfaz está pensada para el borde superior del monitor principal.
- **Capturas:** abarcan todo el escritorio virtual (todos los monitores).
- **Clima:** requiere conexión; la ubicación por IP es aproximada.
- **Letras:** dependen de un servicio en línea y no siempre existen para todas las canciones.
- **Corazón de Spotify:** depende del atajo de la aplicación de escritorio y no refleja el estado real del "me gusta".
- **EPUB:** solo texto, sin imágenes ni CSS, y sin soporte de DRM. La paginación se recalcula con la fuente y la escala actuales.
- **Cuentagotas:** la pantalla se lee con GDI; algunos contenidos protegidos o aceleración por hardware pueden devolver negro.
- **El atajo `Ctrl+Alt+S` no responde:** otra aplicación lo tiene reservado. Usa el menú de la bandeja.
- **No aparecen aplicaciones en el mezclador:** solo se listan las que tienen una sesión de audio en Windows; si una aplicación no ha reproducido sonido todavía, puede no aparecer hasta que lo haga.
- **Error de compilación "Acceso denegado":** la isla sigue en ejecución; ciérrala antes de recompilar.
- **Restablecer todo:** cierra la isla y borra `%APPDATA%\DynamicIsland\`.
