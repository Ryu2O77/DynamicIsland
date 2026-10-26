THIS PROJECT WAS MADE WITH AI FOR PERSONAL USE!

# Dynamic Island para Windows

Una "dynamic island" ligera para Windows hecha en Rust: una píldora negra pegada al borde superior de la pantalla que se expande al pasar el mouse y muestra música, reloj, temporizador, cronómetro y calculadora.

> **Estado:** en desarrollo. Pensada para Windows 10/11.

## Funciones

- **Música**: título, artista, carátula, barra de progreso (clic para saltar), anterior / play-pausa / siguiente y un visualizador animado. Funciona con cualquier reproductor que se integre con los controles multimedia de Windows (Spotify, navegadores, etc.).
- **Reloj**: hora, fecha y batería.
- **Temporizador**: presets de 1, 5, 10 y 25 min, pausa, reiniciar y +1 min. La cuenta se ve también con la isla colapsada.
- **Cronómetro**: décimas de segundo y vueltas.
- **Calculadora**: teclas clicables, paréntesis, resultado en vivo. Intenta aceptar teclado cuando la ventana consigue el foco.
- **Avisos de batería**: al conectar/desconectar el cargador y cuando baja del 20 %.
- **Volumen**: rueda del mouse sobre la isla; clic central para silenciar.
- **Ocultar en pantalla completa** (juegos, videos).
- **Icono en la bandeja** con menú para configurar y salir.
- **Ajustes**: cada función se puede activar o desactivar desde la pestaña *Ajustes* o desde el menú de la bandeja.

## Uso

Pasa el mouse sobre la isla para expandirla y usa la barra de pestañas (**Música · Reloj · Timer · Crono · Calc · Ajustes**). El punto del extremo derecho la mantiene abierta (*pin*). Clic derecho sobre la isla o sobre el icono de la bandeja abre el menú.

La configuración se guarda en `%APPDATA%\DynamicIsland\config.ini`.

## Descargar

Los ejecutables compilados están en la sección **Releases** del repositorio (si hay alguna publicada).

## Compilar

Requisitos: [Rust](https://rustup.rs) y las herramientas de compilación de C++ de Visual Studio (Build Tools).

```powershell
git clone <url-del-repo>
cd DynamicIsland
cargo build --release
.\target\release\dynamic_island.exe
```

El ejecutable queda en `target\release\dynamic_island.exe`. Para cerrarlo: clic derecho en la isla o en el icono de la bandeja y *Salir*.

## Cómo funciona

- Una sola ventana Win32 "layered" con alpha por píxel, sin frameworks de UI.
- Dibujo por software con bordes suavizados (distancias con signo) y texto con GDI.
- La animación es un resorte sincronizado con el compositor (DWM) y solo corre mientras hay movimiento; en reposo el consumo es prácticamente cero.
- La música se lee con la API de controles multimedia del sistema (WinRT).

## Publicar una versión

El workflow de `.github/workflows/build.yml` compila en cada push y, al subir un tag que empiece por `v`, adjunta el `.exe` a una release:

```powershell
git tag v0.4.0
git push origin v0.4.0
```

## Licencia

[MIT](LICENSE)
