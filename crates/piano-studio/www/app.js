"use strict";

// piano studio — browser control surface.
//
// Talks to the server described in docs/PARAMETER-STUDIO.md: GET /api/piano
// for the full resolved state and its slider ranges, POST /api/live for
// one change at a time (playing a note is a change like any other), POST
// /api/save and /api/load for files, and GET /api/live upgraded to an SSE
// stream so every open tab stays in sync with what the others just did.
//
// There is no client-side framework here — the page is small enough that
// hand-written DOM code stays easier to follow than a build step would be.

const state = {
  snapshot: null,
  selectedMidi: null,
  selectedStringIndex: 0,
  // Shift-click builds a set of whole keys to edit together. A plain click
  // always clears it, so "selection" never lingers by surprise.
  selection: new Set(),
  octaveShift: 0,
};

// Maps a computer-keyboard character to a semitone offset from the row's
// first key, following a standard "piano key" layout: white keys along the
// home row, black keys on the row above.
const KEYBOARD_SEMITONES = {
  a: 0, w: 1, s: 2, e: 3, d: 4, f: 5, t: 6, g: 7, y: 8, h: 9, u: 10, j: 11,
  k: 12, o: 13, l: 14, p: 15, ";": 16,
};

const BASE_OCTAVE_MIDI = 60; // C4, where the computer-keyboard row starts.
const heldComputerKeys = new Map(); // character -> midi, so key-up releases the right note.

function main() {
  wireHeader();
  wirePlayControls();
  wireComputerKeyboard();
  wireStringScope();
  loadSnapshot().then(() => {
    renderAll();
    connectEventStream();
  });
}

// ---- Data loading -----------------------------------------------------

async function loadSnapshot() {
  const response = await fetch("/api/piano");
  state.snapshot = await response.json();
}

async function postEdit(edit) {
  const response = await fetch("/api/live", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(edit),
  });
  if (!response.ok) {
    setStatus(`edit refused: ${await response.text()}`);
  }
}

function connectEventStream() {
  const source = new EventSource("/api/live");
  source.onmessage = (message) => handleServerEvent(JSON.parse(message.data));
  source.onerror = () => setStatus("disconnected — retrying…");
}

// An event from `EventSource` is either an echo of an edit another tab
// made, or one of the two the server raises itself (`reload`, `saved`).
// Re-fetching the whole snapshot on every message is simpler than trying
// to apply each edit shape twice (once server-side, once here) and keeps
// this file from drifting out of sync with `crate::edit::Edit`.
function handleServerEvent(event) {
  if (event.type === "saved") {
    setStatus(`saved to ${event.path}`);
    return;
  }
  loadSnapshot().then(renderAll);
}

// ---- Header: file path, save, load -------------------------------------

function wireHeader() {
  document.getElementById("save").addEventListener("click", () => {
    const path = document.getElementById("path").value.trim();
    save(path.length > 0 ? path : undefined);
  });
  document.getElementById("load").addEventListener("click", () => {
    const path = document.getElementById("path").value.trim();
    if (path.length === 0) {
      setStatus("enter a path to load");
      return;
    }
    load(path);
  });
}

async function save(path) {
  const response = await fetch("/api/save", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ path }),
  });
  if (!response.ok) {
    setStatus(`save failed: ${await response.text()}`);
    return;
  }
  setStatus("saved");
}

async function load(path) {
  const response = await fetch("/api/load", {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ path }),
  });
  if (!response.ok) {
    setStatus(`load failed: ${await response.text()}`);
    return;
  }
  await loadSnapshot();
  renderAll();
  setStatus("loaded");
}

function setStatus(message) {
  document.getElementById("status").textContent = message;
}

// ---- Play controls: velocity, pedal, panic -----------------------------

function wirePlayControls() {
  const velocity = document.getElementById("velocity");
  const velocityValue = document.getElementById("velocity-value");
  velocity.addEventListener("input", () => {
    velocityValue.textContent = Number(velocity.value).toFixed(2);
  });

  document.getElementById("pedal").addEventListener("change", (event) => {
    postEdit({ type: "sustain_pedal", down: event.target.checked });
  });

  document.getElementById("panic").addEventListener("click", () => {
    postEdit({ type: "all_notes_off" });
  });

  document.getElementById("clear-selection").addEventListener("click", () => {
    state.selection.clear();
    renderKeyboard();
    renderStringPanel();
  });
}

function currentVelocity() {
  return Number(document.getElementById("velocity").value);
}

function playNote(midi) {
  postEdit({ type: "note_on", midi, velocity: currentVelocity() });
}

function releaseNote(midi) {
  postEdit({ type: "note_off", midi });
}

// ---- Computer keyboard: play notes, z/x shift octave, space is pedal ---

function wireComputerKeyboard() {
  window.addEventListener("keydown", (event) => {
    if (event.repeat) {
      return;
    }
    if (event.code === "Space") {
      event.preventDefault();
      document.getElementById("pedal").checked = true;
      postEdit({ type: "sustain_pedal", down: true });
      return;
    }
    const key = event.key.toLowerCase();
    if (key === "z") {
      state.octaveShift -= 1;
      return;
    }
    if (key === "x") {
      state.octaveShift += 1;
      return;
    }
    const semitone = KEYBOARD_SEMITONES[key];
    if (semitone === undefined || heldComputerKeys.has(key)) {
      return;
    }
    const midi = BASE_OCTAVE_MIDI + state.octaveShift * 12 + semitone;
    heldComputerKeys.set(key, midi);
    playNote(midi);
    highlightKey(midi, true);
  });

  window.addEventListener("keyup", (event) => {
    if (event.code === "Space") {
      document.getElementById("pedal").checked = false;
      postEdit({ type: "sustain_pedal", down: false });
      return;
    }
    const key = event.key.toLowerCase();
    const midi = heldComputerKeys.get(key);
    if (midi === undefined) {
      return;
    }
    heldComputerKeys.delete(key);
    releaseNote(midi);
    highlightKey(midi, false);
  });
}

function highlightKey(midi, pressed) {
  const element = document.querySelector(`.key[data-midi="${midi}"]`);
  if (element) {
    element.classList.toggle("pressed", pressed);
  }
}

// ---- Keyboard drawing and selection -------------------------------------

function renderAll() {
  document.getElementById("piano-name").textContent = state.snapshot.name
    ? ` — ${state.snapshot.name}`
    : "";
  document.getElementById("path").value = state.snapshot.path || "";
  renderKeyboard();
  renderStringPanel();
  renderInstrumentPanel();
}

function renderKeyboard() {
  const keyboard = document.getElementById("keyboard");
  keyboard.innerHTML = "";
  for (const key of state.snapshot.keys) {
    const element = document.createElement("button");
    element.type = "button";
    element.className = `key ${key.sharp ? "sharp" : "natural"}`;
    element.dataset.midi = String(key.midi);
    element.title = key.name;
    element.classList.toggle("selected", state.selection.has(key.midi));
    element.classList.toggle("active", state.selectedMidi === key.midi);
    element.addEventListener("mousedown", () => playNote(key.midi));
    element.addEventListener("mouseup", () => releaseNote(key.midi));
    element.addEventListener("mouseleave", () => releaseNote(key.midi));
    element.addEventListener("click", (event) => selectKey(key.midi, event.shiftKey));
    keyboard.appendChild(element);
  }
  document.getElementById("selection-count").textContent = String(state.selection.size);
}

function selectKey(midi, additive) {
  if (additive) {
    if (state.selection.has(midi)) {
      state.selection.delete(midi);
    } else {
      state.selection.add(midi);
    }
  } else {
    state.selection.clear();
  }
  state.selectedMidi = midi;
  state.selectedStringIndex = 0;
  renderKeyboard();
  renderStringPanel();
}

// ---- String panel: tabs for unison strings, one slider per parameter ---

// Mirrors `piano_studio::edit::STRING_PARAMETERS` — kept in the same
// order the server serves ranges in, so a slider added there only needs a
// matching label added here.
const STRING_PARAMETERS = [
  { key: "damping", label: "damping" },
  { key: "sustain", label: "sustain" },
  { key: "inharmonicity", label: "inharmonicity" },
  { key: "detune_cents", label: "detune (cents)" },
  { key: "seed", label: "seed" },
  { key: "hammer_contact_exponent", label: "hammer contact exponent" },
  { key: "hammer_stiffness", label: "hammer stiffness" },
  { key: "hammer_mass", label: "hammer mass" },
];

function wireStringScope() {
  for (const radio of document.querySelectorAll('input[name="scope"]')) {
    radio.addEventListener("change", renderStringPanel);
  }
}

function currentScope() {
  return document.querySelector('input[name="scope"]:checked').value;
}

function selectedKey() {
  return state.snapshot.keys.find((key) => key.midi === state.selectedMidi);
}

function renderStringPanel() {
  const title = document.getElementById("string-title");
  const tabs = document.getElementById("string-tabs");
  const sliders = document.getElementById("string-sliders");
  tabs.innerHTML = "";
  sliders.innerHTML = "";

  const key = selectedKey();
  if (!key) {
    title.textContent = "no key selected";
    return;
  }
  title.textContent = `${key.name} — ${key.strings.length} string${key.strings.length === 1 ? "" : "s"}`;

  for (const string of key.strings) {
    const tab = document.createElement("button");
    tab.type = "button";
    tab.className = "tab";
    tab.textContent = `string ${string.string_index + 1}`;
    tab.classList.toggle("active", string.string_index === state.selectedStringIndex);
    tab.addEventListener("click", () => {
      state.selectedStringIndex = string.string_index;
      renderStringPanel();
    });
    tabs.appendChild(tab);
  }

  const string = key.strings.find((candidate) => candidate.string_index === state.selectedStringIndex)
    || key.strings[0];
  if (!string) {
    return;
  }
  const ranges = state.snapshot.ranges.strings;
  for (const parameter of STRING_PARAMETERS) {
    sliders.appendChild(
      buildSlider(parameter.label, ranges[parameter.key], string[parameter.key], (value) => {
        applyStringEdit(key.midi, parameter.key, value);
      }),
    );
  }
}

function applyStringEdit(midi, parameter, value) {
  const scope = currentScope();
  if (scope === "string") {
    postEdit({
      type: "set_string",
      midi,
      string_index: state.selectedStringIndex,
      parameter,
      value,
    });
    return;
  }
  if (scope === "key") {
    const key = state.snapshot.keys.find((candidate) => candidate.midi === midi);
    const strings = key.strings.map((string) => ({ midi, string_index: string.string_index }));
    postEdit({ type: "set_strings", strings, parameter, value });
    return;
  }
  // scope === "selection": every string of every selected key.
  const strings = [];
  for (const selectedMidi of state.selection) {
    const key = state.snapshot.keys.find((candidate) => candidate.midi === selectedMidi);
    for (const string of key.strings) {
      strings.push({ midi: selectedMidi, string_index: string.string_index });
    }
  }
  postEdit({ type: "set_strings", strings, parameter, value });
}

// ---- Instrument panel: soundboard modes, bridge coupling gains ---------

const MODE_PARAMETERS = [
  { key: "frequency_hz", label: "frequency (Hz)" },
  { key: "decay_seconds", label: "decay (s)" },
  { key: "gain", label: "gain" },
];

const BRIDGE_PARAMETERS = [
  { key: "local_coupling_gain", label: "local coupling gain" },
  { key: "global_coupling_gain", label: "global coupling gain" },
];

function renderInstrumentPanel() {
  const modes = document.getElementById("modes");
  modes.innerHTML = "";
  const modeRanges = state.snapshot.ranges.modes;
  for (const mode of state.snapshot.modes) {
    const card = document.createElement("div");
    card.className = "mode-card";
    const heading = document.createElement("h3");
    heading.textContent = `mode ${mode.index + 1}`;
    card.appendChild(heading);
    for (const parameter of MODE_PARAMETERS) {
      card.appendChild(
        buildSlider(parameter.label, modeRanges[parameter.key], mode[parameter.key], (value) => {
          postEdit({ type: "set_mode", index: mode.index, parameter: parameter.key, value });
        }),
      );
    }
    modes.appendChild(card);
  }

  const bridge = document.getElementById("bridge");
  bridge.innerHTML = "";
  const bridgeRanges = state.snapshot.ranges.bridge;
  for (const parameter of BRIDGE_PARAMETERS) {
    bridge.appendChild(
      buildSlider(
        parameter.label,
        bridgeRanges[parameter.key],
        state.snapshot.bridge[parameter.key],
        (value) => {
          postEdit({ type: "set_bridge", parameter: parameter.key, value });
        },
      ),
    );
  }
}

// ---- Shared slider widget ------------------------------------------------

function buildSlider(label, range, value, onChange) {
  const wrapper = document.createElement("label");
  wrapper.className = "slider";

  const caption = document.createElement("span");
  caption.textContent = label;
  wrapper.appendChild(caption);

  const input = document.createElement("input");
  input.type = "range";
  input.min = String(range.low);
  input.max = String(range.high);
  input.step = String(range.step);
  input.value = String(value);
  wrapper.appendChild(input);

  const output = document.createElement("output");
  output.textContent = formatValue(value);
  wrapper.appendChild(output);

  input.addEventListener("input", () => {
    output.textContent = formatValue(Number(input.value));
  });
  input.addEventListener("change", () => {
    onChange(Number(input.value));
  });

  return wrapper;
}

function formatValue(value) {
  if (Math.abs(value) >= 1000 || (value !== 0 && Math.abs(value) < 0.001)) {
    return value.toExponential(2);
  }
  return value.toFixed(3);
}

main();
