const canvas = document.getElementById("game");
const ctx = canvas.getContext("2d");
const scoreEl = document.getElementById("score");
const humansEl = document.getElementById("humans");
const enemiesEl = document.getElementById("enemies");
const overlayEl = document.getElementById("overlay");
const finalScoreEl = document.getElementById("final-score");
const restartBtn = document.getElementById("restart-btn");

const BIT_FIRE = 1;
const BIT_LEFT = 2;
const BIT_UP = 4;
const BIT_RIGHT = 8;
const BIT_DOWN = 16;

let inputBits = 0;

function keyToBit(code) {
  switch (code) {
    case "Space":
      return BIT_FIRE;
    case "ArrowLeft":
    case "KeyA":
      return BIT_LEFT;
    case "ArrowUp":
    case "KeyW":
      return BIT_UP;
    case "ArrowRight":
    case "KeyD":
      return BIT_RIGHT;
    case "ArrowDown":
    case "KeyS":
      return BIT_DOWN;
    default:
      return 0;
  }
}

async function loadWasm(url) {
  if (WebAssembly.instantiateStreaming) {
    try {
      const { instance } = await WebAssembly.instantiateStreaming(fetch(url));
      return instance.exports;
    } catch {
      // Fallback if server doesn't serve application/wasm MIME type
    }
  }
  const response = await fetch(url);
  const bytes = await response.arrayBuffer();
  const { instance } = await WebAssembly.instantiate(bytes);
  return instance.exports;
}

async function main() {
  const wasm = await loadWasm("./defender.wasm");
  wasm.init();

  const w = wasm.width();
  const h = wasm.height();
  canvas.width = w;
  canvas.height = h;

  const imageData = ctx.createImageData(w, h);

  function restartGame() {
    inputBits = 0;
    wasm.reset();
    wasm.set_input(0);
    overlayEl.classList.add("hidden");
  }

  restartBtn.addEventListener("click", restartGame);

  window.addEventListener("keydown", (event) => {
    if (event.code === "KeyR" || (event.code === "Enter" && wasm.is_game_over())) {
      restartGame();
      return;
    }
    const bit = keyToBit(event.code);
    if (bit) {
      event.preventDefault();
      inputBits |= bit;
      wasm.set_input(inputBits);
    }
  });

  window.addEventListener("keyup", (event) => {
    const bit = keyToBit(event.code);
    if (bit) {
      event.preventDefault();
      inputBits &= ~bit;
      wasm.set_input(inputBits);
    }
  });

  window.addEventListener("blur", () => {
    inputBits = 0;
    wasm.set_input(0);
  });

  const STEP_MS = 16;
  let lastTime = performance.now();
  let accumulator = 0;

  function frame(now) {
    const dt = Math.min(now - lastTime, 100);
    lastTime = now;
    accumulator += dt;

    while (accumulator >= STEP_MS) {
      if (!wasm.is_game_over()) {
        wasm.tick();
      }
      accumulator -= STEP_MS;
    }

    const ptr = wasm.framebuffer_ptr();
    const len = wasm.framebuffer_len();
    const view = new Uint8ClampedArray(wasm.memory.buffer, ptr, len);
    imageData.data.set(view);
    ctx.putImageData(imageData, 0, 0);

    const currentScore = wasm.score();
    scoreEl.textContent = String(currentScore);
    humansEl.textContent = String(wasm.humans_alive());
    enemiesEl.textContent = String(wasm.enemies_alive());

    if (wasm.is_game_over()) {
      finalScoreEl.textContent = String(currentScore);
      overlayEl.classList.remove("hidden");
    }

    requestAnimationFrame(frame);
  }

  requestAnimationFrame(frame);
}

main().catch((err) => {
  console.error("Failed to start Defender WASM:", err);
});
