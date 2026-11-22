const canvas = document.getElementById("game");
const ctx = canvas.getContext("2d");
const scoreEl = document.getElementById("score");
const livesEl = document.getElementById("lives");
const humansEl = document.getElementById("humans");
const enemiesEl = document.getElementById("enemies");
const overlayEl = document.getElementById("overlay");
const overlayTitleEl = document.getElementById("overlay-title");
const overlayMessageEl = document.getElementById("overlay-message");
const overlayBtn = document.getElementById("overlay-btn");
const pauseBtn = document.getElementById("pause-btn");

const BIT_FIRE = 1;
const BIT_LEFT = 2;
const BIT_UP = 4;
const BIT_RIGHT = 8;
const BIT_DOWN = 16;

let inputBits = 0;
let isPaused = false;

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

  const STEP_MS = 16;
  let lastTime = performance.now();
  let accumulator = 0;

  function showGameOver(currentScore) {
    overlayTitleEl.textContent = "GAME OVER";
    overlayMessageEl.innerHTML = `Final Score: <strong>${currentScore}</strong>`;
    overlayBtn.textContent = "PLAY AGAIN (R)";
    overlayEl.classList.remove("hidden");
  }

  function togglePause() {
    if (wasm.is_game_over()) return;
    isPaused = !isPaused;
    if (isPaused) {
      inputBits = 0;
      wasm.set_input(0);
      pauseBtn.textContent = "RESUME (P)";
      overlayTitleEl.textContent = "PAUSED";
      overlayMessageEl.textContent = "Press P or click Resume to continue";
      overlayBtn.textContent = "RESUME (P)";
      overlayEl.classList.remove("hidden");
    } else {
      pauseBtn.textContent = "PAUSE (P)";
      overlayEl.classList.add("hidden");
      lastTime = performance.now();
      accumulator = 0;
    }
  }

  function restartGame() {
    isPaused = false;
    inputBits = 0;
    wasm.reset();
    wasm.set_input(0);
    pauseBtn.textContent = "PAUSE (P)";
    overlayEl.classList.add("hidden");
    lastTime = performance.now();
    accumulator = 0;
  }

  overlayBtn.addEventListener("click", () => {
    if (isPaused && !wasm.is_game_over()) {
      togglePause();
    } else {
      restartGame();
    }
  });
  pauseBtn.addEventListener("click", togglePause);

  window.addEventListener("keydown", (event) => {
    if (event.code === "KeyP" && !event.repeat) {
      event.preventDefault();
      togglePause();
      return;
    }
    if (event.code === "KeyR" || (event.code === "Enter" && wasm.is_game_over())) {
      event.preventDefault();
      restartGame();
      return;
    }
    if (isPaused) return;
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

  function frame(now) {
    const dt = Math.min(now - lastTime, 100);
    lastTime = now;

    if (!isPaused && !wasm.is_game_over()) {
      accumulator += dt;
      while (accumulator >= STEP_MS) {
        if (!wasm.is_game_over()) {
          wasm.tick();
        }
        accumulator -= STEP_MS;
      }
    }

    const ptr = wasm.framebuffer_ptr();
    const len = wasm.framebuffer_len();
    const view = new Uint8ClampedArray(wasm.memory.buffer, ptr, len);
    imageData.data.set(view);
    ctx.putImageData(imageData, 0, 0);

    const currentScore = wasm.score();
    scoreEl.textContent = String(currentScore);
    livesEl.textContent = String(wasm.lives());
    humansEl.textContent = String(wasm.humans_alive());
    enemiesEl.textContent = String(wasm.enemies_alive());

    if (!isPaused && wasm.is_game_over()) {
      showGameOver(currentScore);
    }

    requestAnimationFrame(frame);
  }

  requestAnimationFrame(frame);
}

main().catch((err) => {
  console.error("Failed to start Defender WASM:", err);
});
