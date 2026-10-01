# First-Person Web Prototype

A deliberately small interaction prototype based on the native first-person lab. It uses
standalone WebGL2 with desktop pointer lock and touch controls so it can run without a WASM target,
JavaScript package manager, bundler, Woven node, or hosted service.

This is not the Weaver runtime. Its room dimensions, eye height, movement
speed, mouse sensitivity, and movement bounds mirror
`../../examples/first-person-lab`.

## Run

From the Weaver repository root:

```bash
python3 -m http.server 8000 --directory prototypes/first-person-web
```

Then open <http://127.0.0.1:8000>. Stop the server with `Ctrl+C`.

The page makes no network requests after its three local files are loaded.

## Controls

Desktop:

- Click the canvas or **Enter room** to capture the pointer.
- Mouse: look.
- `WASD` or arrow keys: move.
- `Escape`: release the pointer.

Mobile/coarse-pointer devices:

- Tap **Enter room** or the scene to begin.
- Drag the scene to look.
- Hold the lower-left movement-pad buttons to walk.

Movement remains at standing eye height and is clamped to the room interior.
WebGL2 is required; pointer lock is only required for desktop mouse look.
