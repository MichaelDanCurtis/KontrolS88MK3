const state = {
  revision: 0,
  serverGraph: { nodes: [] },
  graph: { nodes: [] },
  meters: new Map(),
  positions: new Map(),
  modules: [],
  pendingCable: null,
  dirty: false,
  rackSnapshot: null,
  rackProposal: null,
  pointerInteraction: false,
};

const workspace = document.querySelector('#workspace');
const wires = document.querySelector('#wires');
const applyButton = document.querySelector('#apply');
const dirtyLabel = document.querySelector('#dirty');
const notice = document.querySelector('#notice');
const moduleSearch = document.querySelector('#module-search');
const moduleCategory = document.querySelector('#module-category');
const modulePalette = document.querySelector('#module-palette');
const rackStatus = document.querySelector('#rack-status');
const rackInventory = document.querySelector('#rack-inventory');
const copilotProvider = document.querySelector('#copilot-provider');
const copilotPrompt = document.querySelector('#copilot-prompt');
const copilotAsk = document.querySelector('#copilot-ask');
const copilotProposal = document.querySelector('#copilot-proposal');
const copilotNotice = document.querySelector('#copilot-notice');
const copilotApply = document.querySelector('#copilot-apply');
const copilotUndo = document.querySelector('#copilot-undo');

const clone = value => JSON.parse(JSON.stringify(value));
const kindOf = spec => Object.keys(spec)[0];
const payloadOf = spec => spec[kindOf(spec)];
const esc = value => String(value).replace(/[&<>'"]/g, char => ({'&':'&amp;','<':'&lt;','>':'&gt;',"'":'&#39;','"':'&quot;'}[char]));

function defaultSpec(kind) {
  if (kind === 'lfo') return { Lfo: { shape: 'triangle', freq_hz: 0.3 } };
  if (kind === 'sample-hold') return { SampleHold: { rate_hz: 2, seed: 1 } };
  if (kind === 'step-sequence') return { StepSequence: { rate_hz: 4, values: [0, 0.35, 0.75, 1] } };
  if (kind === 'slew') return { Slew: { rise_per_second: 1, fall_per_second: 1 } };
  if (kind === 'clamp') return { Clamp: { min: 0, max: 1 } };
  if (kind === 'curve') return { Curve: { exponent: 2 } };
  if (kind === 'quantize') return { Quantize: { steps: 8 } };
  if (kind === 'math') return { Math: { operation: 'multiply', operand: 1 } };
  if (kind === 'envelope') return { Envelope: { gate: '', attack_seconds: 0.05, decay_seconds: 0.2, sustain: 0.7, release_seconds: 0.4 } };
  if (kind === 'macro') return { Macro: { binding: 's88.knob.1', default: 0 } };
  if (kind === 'scale') return { Scale: { min: 0, max: 127 } };
  if (kind === 'external-source') return { ExternalSource: { binding: 'osc.input', default: 0 } };
  if (kind === 'external-sink') return { ExternalSink: { binding: 'osc.output' } };
  if (kind === 'takeover') return { Takeover: { control: '', pickup_epsilon: 0.01 } };
  return { Sink: { device: 'peak', param: 'filter.cutoff' } };
}

function portsForSpec(spec) {
  const kind = kindOf(spec);
  const input = (id, name, signal = 'control') => ({ id, name, direction: 'input', signal, required: true });
  const output = { id: 'out', name: 'OUT', direction: 'output', signal: 'control', required: false };
  if (['Lfo', 'SampleHold', 'StepSequence', 'Macro', 'ExternalSource'].includes(kind)) return [output];
  if (kind === 'Envelope') return [input('gate', 'GATE', 'gate'), output];
  if (['Sink', 'ExternalSink'].includes(kind)) return [input('signal', 'IN')];
  if (kind === 'Takeover') return [input('signal', 'IN'), input('control', 'PICKUP'), output];
  return [input('signal', 'IN'), output];
}

function fallbackModules() {
  return [
    ['lfo', 'LFO'], ['sample-hold', 'Sample & Hold'], ['step-sequence', 'Step Sequence'],
    ['slew', 'Slew'], ['clamp', 'Clamp'], ['curve', 'Curve'], ['quantize', 'Quantize'],
    ['math', 'Math'], ['envelope', 'Envelope'], ['macro', 'Macro'], ['scale', 'Scale'],
    ['sink', 'Device Out'], ['external-source', 'Control In'],
    ['external-sink', 'Signal Out'], ['takeover', 'Pickup'],
  ].map(([id, name]) => {
    const default_spec = defaultSpec(id);
    return { id, name, category: 'Core', summary: kindOf(default_spec), default_spec, ports: portsForSpec(default_spec) };
  });
}

function renderModuleLibrary() {
  const query = moduleSearch.value.trim().toLowerCase();
  const category = moduleCategory.value;
  const filtered = state.modules.filter(module =>
    (category === 'All' || module.category === category)
    && (!query || `${module.name} ${module.summary} ${module.category}`.toLowerCase().includes(query))
  );
  document.querySelector('#module-count').textContent = `${filtered.length}/${state.modules.length}`;
  modulePalette.innerHTML = filtered.map(module => {
    const inputCount = module.ports.filter(port => port.direction === 'input').length;
    const outputCount = module.ports.filter(port => port.direction === 'output').length;
    return `<button class="palette" data-module="${esc(module.id)}"><strong>＋ ${esc(module.name)}</strong><span class="port-count">${inputCount}↦${outputCount}</span><small>${esc(module.summary)}</small></button>`;
  }).join('') || '<p class="muted">No matching modules.</p>';
  modulePalette.querySelectorAll('[data-module]').forEach(button => button.addEventListener('click', () => {
    const module = state.modules.find(candidate => candidate.id === button.dataset.module);
    if (module) addModule(module);
  }));
}

function acceptCatalog(catalog) {
  state.modules = catalog.modules?.length ? catalog.modules : fallbackModules();
  const categories = ['All', ...new Set(state.modules.map(module => module.category))];
  moduleCategory.innerHTML = categories.sort((a, b) => a === 'All' ? -1 : b === 'All' ? 1 : a.localeCompare(b)).map(value => `<option>${esc(value)}</option>`).join('');
  renderModuleLibrary();
}

function nextId(kind) {
  const base = kind === 'sink' ? 'output' : kind.replace('external-', 'osc-');
  let number = 1;
  while (state.graph.nodes.some(node => node.id === `${base}-${number}`)) number += 1;
  return `${base}-${number}`;
}

function setDirty(dirty = true) {
  state.dirty = dirty;
  applyButton.disabled = !dirty;
  dirtyLabel.textContent = dirty ? 'Unapplied local changes' : 'Patch is synchronized';
  dirtyLabel.classList.toggle('changed', dirty);
}

function showNotice(message, type = 'error') {
  notice.textContent = message;
  notice.className = type;
}

function clearNotice() {
  notice.textContent = '';
  notice.className = '';
}

async function apiJson(url, options = {}) {
  const response = await fetch(url, options);
  const body = await response.json().catch(() => ({}));
  if (!response.ok) throw new Error(body.error || `Request failed (${response.status})`);
  return body;
}

function showCopilotNotice(message, type = 'error') {
  copilotNotice.textContent = message;
  copilotNotice.className = type;
}

function clearCopilotNotice() {
  copilotNotice.textContent = '';
  copilotNotice.className = '';
}

async function refreshRackSnapshot() {
  rackStatus.className = 'rack-orb scanning';
  try {
    const snapshot = await apiJson('/api/v1/rack/snapshot');
    state.rackSnapshot = snapshot;
    rackStatus.className = 'rack-orb online';
    const pluginCount = new Set(snapshot.catalog.map(model => model.plugin)).size;
    rackInventory.textContent = `${snapshot.patch.modules.length} in patch · ${snapshot.catalog.length} models · ${pluginCount} plugins`;
    rackInventory.className = 'inventory-chip online';
  } catch (error) {
    state.rackSnapshot = null;
    rackStatus.className = 'rack-orb';
    rackInventory.textContent = 'Add Bridge8 to an open VCV Rack patch';
    rackInventory.className = 'inventory-chip';
  }
}

function commandLabel(command) {
  if (command.type === 'add_module') return `Add ${command.plugin} / ${command.model} as ${command.reference}`;
  if (command.type === 'set_param') return `Set parameter ${command.param_id} on ${command.module} to ${Number(command.value).toPrecision(4)}`;
  if (command.type === 'connect') return `Patch ${command.output_module} [${command.output_id}] → ${command.input_module} [${command.input_id}]`;
  return command.type;
}

function renderRackProposal(proposal) {
  state.rackProposal = proposal;
  const provider = (proposal.provider || 'openai').toUpperCase();
  document.querySelector('#copilot-model').textContent = `${provider} · ${proposal.model}`;
  document.querySelector('#copilot-summary').textContent = proposal.summary;
  const warnings = document.querySelector('#copilot-warnings');
  warnings.innerHTML = proposal.warnings.length
    ? proposal.warnings.map(warning => `<p>⚠ ${esc(warning)}</p>`).join('')
    : '';
  document.querySelector('#copilot-commands').innerHTML = proposal.commands
    .map(command => `<li><span class="command-type">${esc(command.type.replace('_', ' '))}</span>${esc(commandLabel(command))}</li>`)
    .join('');
  const suggestions = document.querySelector('#copilot-suggestions');
  suggestions.innerHTML = proposal.library_suggestions.length
    ? `<h4>Library ideas — not installed</h4>${proposal.library_suggestions.map(item => `<article><strong>${esc(item.query)}</strong><p>${esc(item.reason)}</p></article>`).join('')}`
    : '';
  copilotProposal.hidden = false;
  copilotApply.disabled = false;
}

async function proposeRackPatch() {
  const prompt = copilotPrompt.value.trim();
  if (!prompt) { showCopilotNotice('Describe what you want Rack to build.'); return; }
  clearCopilotNotice();
  copilotAsk.disabled = true;
  copilotAsk.innerHTML = '<span>Inspecting Rack + planning…</span><b>◌</b>';
  copilotProposal.hidden = true;
  state.rackProposal = null;
  try {
    const proposal = await apiJson('/api/v1/rack/copilot/propose', {
      method: 'POST', headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ prompt, provider: copilotProvider.value || undefined }),
    });
    renderRackProposal(proposal);
    showCopilotNotice('Proposal validated against the live Rack inventory. Review it before applying.', 'success');
  } catch (error) {
    showCopilotNotice(error.message);
    await refreshRackSnapshot();
  } finally {
    copilotAsk.disabled = false;
    copilotAsk.innerHTML = '<span>Draft patch architecture</span><b>✦</b>';
  }
}

async function applyRackProposal() {
  if (!state.rackProposal) return;
  copilotApply.disabled = true;
  copilotApply.textContent = 'Applying in Rack…';
  try {
    const result = await apiJson('/api/v1/rack/copilot/apply', {
      method: 'POST', headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ proposal_id: state.rackProposal.id }),
    });
    showCopilotNotice(result.message, 'success');
    state.rackProposal = null;
    copilotProposal.hidden = true;
    copilotUndo.disabled = false;
    await refreshRackSnapshot();
  } catch (error) {
    showCopilotNotice(error.message);
    copilotApply.disabled = false;
  } finally {
    copilotApply.textContent = 'Apply in Rack';
  }
}

async function undoRackPatch() {
  copilotUndo.disabled = true;
  try {
    const result = await apiJson('/api/v1/rack/copilot/undo', { method: 'POST' });
    showCopilotNotice(result.message, 'success');
    await refreshRackSnapshot();
  } catch (error) {
    showCopilotNotice(error.message);
  }
}

function ensurePositions() {
  const columnWidth = 216;
  const columns = Math.max(1, Math.floor((workspace.clientWidth - 20) / columnWidth));
  state.graph.nodes.forEach((node, index) => {
    if (!state.positions.has(node.id)) {
      state.positions.set(node.id, {
        x: 18 + (index % columns) * columnWidth,
        y: 45 + Math.floor(index / columns) * 330,
      });
    }
  });
}

function render() {
  ensurePositions();
  workspace.querySelectorAll('.node').forEach(node => node.remove());
  document.querySelector('#empty').hidden = state.graph.nodes.length > 0;
  for (const node of state.graph.nodes) workspace.appendChild(renderNode(node));
  requestAnimationFrame(renderWires);
}

function renderNode(node) {
  const kind = kindOf(node.spec);
  const payload = payloadOf(node.spec);
  const element = document.createElement('article');
  element.className = 'node';
  element.dataset.id = node.id;
  const pos = state.positions.get(node.id);
  element.style.transform = `translate(${pos.x}px, ${pos.y}px)`;

  const upstream = state.graph.nodes.filter(candidate => candidate.id !== node.id);
  const inputOptions = ['<option value="">None</option>', ...upstream.map(candidate =>
    `<option value="${esc(candidate.id)}" ${node.input === candidate.id ? 'selected' : ''}>${esc(candidate.id)}</option>`
  )].join('');
  const controlOptions = ['<option value="">Choose a physical control</option>', ...upstream.map(candidate =>
    `<option value="${esc(candidate.id)}" ${payload.control === candidate.id ? 'selected' : ''}>${esc(candidate.id)}</option>`
  )].join('');

  let fields = '';
  if (kind === 'Lfo') {
    fields = `<label>Shape<select data-field="shape"><option ${payload.shape === 'triangle' ? 'selected' : ''}>triangle</option><option ${payload.shape === 'sine' ? 'selected' : ''}>sine</option></select></label>
      <label>Frequency (Hz)<input data-field="freq_hz" type="number" min="0.001" step="0.01" value="${esc(payload.freq_hz)}"></label>`;
  } else if (kind === 'SampleHold') {
    fields = `<label>Rate (Hz)<input data-field="rate_hz" type="number" min="0.001" step="0.01" value="${esc(payload.rate_hz)}"></label>
      <label>Seed<input data-field="seed" type="number" min="0" max="9007199254740991" step="1" value="${esc(payload.seed)}"></label>`;
  } else if (kind === 'StepSequence') {
    fields = `<label>Rate (steps/s)<input data-field="rate_hz" type="number" min="0.001" step="0.01" value="${esc(payload.rate_hz)}"></label>
      <label>Values (comma separated)<input data-values value="${esc(payload.values.join(', '))}"></label>`;
  } else if (kind === 'Slew') {
    fields = `<label>Rise per second<input data-field="rise_per_second" type="number" min="0.001" step="0.01" value="${esc(payload.rise_per_second)}"></label>
      <label>Fall per second<input data-field="fall_per_second" type="number" min="0.001" step="0.01" value="${esc(payload.fall_per_second)}"></label>`;
  } else if (kind === 'Clamp') {
    fields = `<label>Minimum<input data-field="min" type="number" step="0.01" value="${esc(payload.min)}"></label>
      <label>Maximum<input data-field="max" type="number" step="0.01" value="${esc(payload.max)}"></label>`;
  } else if (kind === 'Curve') {
    fields = `<label>Exponent<input data-field="exponent" type="number" min="0.001" step="0.01" value="${esc(payload.exponent)}"></label>`;
  } else if (kind === 'Quantize') {
    fields = `<label>Levels<input data-field="steps" type="number" min="2" max="65535" step="1" value="${esc(payload.steps)}"></label>`;
  } else if (kind === 'Math') {
    fields = `<label>Operation<select data-field="operation"><option ${payload.operation === 'add' ? 'selected' : ''}>add</option><option ${payload.operation === 'subtract' ? 'selected' : ''}>subtract</option><option ${payload.operation === 'multiply' ? 'selected' : ''}>multiply</option><option ${payload.operation === 'divide' ? 'selected' : ''}>divide</option></select></label>
      <label>Operand<input data-field="operand" type="number" step="0.01" value="${esc(payload.operand)}"></label>`;
  } else if (kind === 'Envelope') {
    const gateOptions = ['<option value="">Choose a gate</option>', ...upstream.map(candidate =>
      `<option value="${esc(candidate.id)}" ${payload.gate === candidate.id ? 'selected' : ''}>${esc(candidate.id)}</option>`
    )].join('');
    fields = `<label>Gate<select data-field="gate">${gateOptions}</select></label>
      <label>Attack (s)<input data-field="attack_seconds" type="number" min="0" step="0.01" value="${esc(payload.attack_seconds)}"></label>
      <label>Decay (s)<input data-field="decay_seconds" type="number" min="0" step="0.01" value="${esc(payload.decay_seconds)}"></label>
      <label>Sustain<input data-field="sustain" type="number" min="0" max="1" step="0.01" value="${esc(payload.sustain)}"></label>
      <label>Release (s)<input data-field="release_seconds" type="number" min="0" step="0.01" value="${esc(payload.release_seconds)}"></label>`;
  } else if (kind === 'Macro') {
    fields = `<label>Adapter binding<input data-field="binding" value="${esc(payload.binding)}"></label>
      <label>Default<input data-field="default" type="number" step="0.01" value="${esc(payload.default)}"></label>`;
  } else if (kind === 'Scale') {
    fields = `<label>Minimum<input data-field="min" type="number" step="0.1" value="${esc(payload.min)}"></label>
      <label>Maximum<input data-field="max" type="number" step="0.1" value="${esc(payload.max)}"></label>`;
  } else if (kind === 'Sink') {
    fields = `<label>Device ID<input data-field="device" value="${esc(payload.device)}"></label>
      <label>Parameter<input data-field="param" value="${esc(payload.param)}"></label>`;
  } else if (kind === 'ExternalSource') {
    fields = `<label>Adapter binding<input data-field="binding" value="${esc(payload.binding)}"></label>
      <label>Default<input data-field="default" type="number" step="0.01" value="${esc(payload.default)}"></label>`;
  } else if (kind === 'ExternalSink') {
    fields = `<label>Adapter binding<input data-field="binding" value="${esc(payload.binding)}"></label>`;
  } else if (kind === 'Takeover') {
    fields = `<label>Physical control<select data-field="control">${controlOptions}</select></label>
      <label>Pickup tolerance<input data-field="pickup_epsilon" type="number" min="0" max="1" step="0.001" value="${esc(payload.pickup_epsilon)}"></label>`;
  }

  const ports = portsForSpec(node.spec);
  let inputIndex = 0;
  const portsMarkup = ports.map(port => {
    const y = port.direction === 'output' ? 52 : 52 + inputIndex++ * 51;
    const connected = isPortConnected(node, port);
    const pending = port.direction === 'output' && state.pendingCable === node.id;
    const classes = ['patch-port', port.direction, port.signal, connected ? 'connected' : '', pending ? 'pending' : ''].filter(Boolean).join(' ');
    const button = `<button class="jack" data-port-id="${esc(port.id)}" data-direction="${port.direction}" title="${esc(port.name)} ${port.direction}"></button>`;
    return `<span class="${classes}" style="--port-y:${y}px">${port.direction === 'output' ? `<span>${esc(port.name)}</span>${button}` : `${button}<span>${esc(port.name)}</span>`}</span>`;
  }).join('');

  element.innerHTML = `${portsMarkup}<div class="node-head"><div><div class="node-kind">${esc(kind)}</div><div class="node-title">${esc(node.id)}</div></div><button class="remove" title="Remove">×</button></div>
    <div class="node-body">
      <label>Node ID<input data-id-field value="${esc(node.id)}"></label>
      ${['Lfo', 'SampleHold', 'StepSequence', 'Envelope', 'Macro', 'ExternalSource'].includes(kind) ? '' : `<label>Input<select data-input>${inputOptions}</select></label>`}
      ${fields}
      <div class="meter"><span></span></div><div class="value">0.000</div>
    </div>`;

  element.querySelector('.remove').addEventListener('click', () => removeNode(node.id));
  element.querySelector('[data-id-field]').addEventListener('change', event => renameNode(node.id, event.target.value.trim()));
  element.querySelector('[data-input]')?.addEventListener('change', event => { node.input = event.target.value || null; setDirty(); render(); });
  element.querySelectorAll('[data-field]').forEach(input => input.addEventListener('change', event => {
    const field = event.target.dataset.field;
    payload[field] = event.target.type === 'number' ? Number(event.target.value) : event.target.value;
    setDirty();
  }));
  element.querySelector('[data-values]')?.addEventListener('change', event => {
    const values = event.target.value.split(',').map(value => Number(value.trim()));
    if (values.length < 1 || values.length > 64 || values.some(value => !Number.isFinite(value))) {
      showNotice('A step sequence needs 1–64 finite, comma-separated values.');
      render();
      return;
    }
    payload.values = values;
    clearNotice();
    setDirty();
  });
  attachPortInteractions(element, node);
  attachDrag(element, node.id);
  updateMeter(element, state.meters.get(node.id) ?? 0);
  return element;
}

function isPortConnected(node, port) {
  if (port.direction === 'output') return true;
  if (port.id === 'signal') return Boolean(node.input);
  if (port.id === 'gate') return Boolean(node.spec.Envelope?.gate);
  if (port.id === 'control') return Boolean(node.spec.Takeover?.control);
  return false;
}

function attachPortInteractions(element, node) {
  element.querySelectorAll('.jack').forEach(jack => {
    let suppressClick = false;
    if (jack.dataset.direction === 'output') {
      jack.addEventListener('pointerdown', downEvent => {
        if (downEvent.button !== 0) return;
        downEvent.preventDefault();
        state.pointerInteraction = true;
        const gesture = {
          pointerId: downEvent.pointerId,
          startX: downEvent.clientX,
          startY: downEvent.clientY,
          moved: false,
        };
        const move = moveEvent => {
          if (moveEvent.pointerId !== gesture.pointerId) return;
          if (!gesture.moved && Math.hypot(moveEvent.clientX - gesture.startX, moveEvent.clientY - gesture.startY) < 5) return;
          gesture.moved = true;
          state.pendingCable = node.id;
          jack.closest('.patch-port').classList.add('pending');
          markCableTarget(moveEvent.clientX, moveEvent.clientY);
          renderCablePreview(jack, moveEvent.clientX, moveEvent.clientY);
        };
        const finish = (upEvent, cancelled = false) => {
          if (upEvent.pointerId !== gesture.pointerId) return;
          window.removeEventListener('pointermove', move);
          window.removeEventListener('pointerup', up);
          window.removeEventListener('pointercancel', cancel);
          clearCableTargets();
          if (!gesture.moved) { state.pointerInteraction = false; return; }
          suppressClick = true;
          const targetJack = cancelled ? null : document.elementFromPoint(upEvent.clientX, upEvent.clientY)?.closest('.jack[data-direction="input"]');
          const targetNode = targetJack ? state.graph.nodes.find(candidate => candidate.id === targetJack.closest('.node')?.dataset.id) : null;
          const connected = targetJack && targetNode
            ? connectCable(node.id, targetNode, targetJack.dataset.portId)
            : false;
          state.pendingCable = null;
          state.pointerInteraction = false;
          if (!connected && !cancelled) showNotice('Cable cancelled — drop on an input jack.');
          render();
        };
        const up = upEvent => finish(upEvent);
        const cancel = cancelEvent => finish(cancelEvent, true);
        window.addEventListener('pointermove', move);
        window.addEventListener('pointerup', up);
        window.addEventListener('pointercancel', cancel);
      });
    }
    jack.addEventListener('click', event => {
      event.preventDefault();
      event.stopPropagation();
      if (suppressClick) { suppressClick = false; return; }
      const portId = jack.dataset.portId;
      if (jack.dataset.direction === 'output') {
        state.pendingCable = state.pendingCable === node.id ? null : node.id;
        showNotice(state.pendingCable ? `Patching from ${node.id} OUT — choose or drag to an input jack.` : 'Cable cancelled.', 'success');
        render();
        return;
      }
      if (state.pendingCable) {
        const source = state.pendingCable;
        state.pendingCable = null;
        if (connectCable(source, node, portId)) render();
        return;
      }
      if (portId === 'signal') node.input = null;
      if (portId === 'gate') node.spec.Envelope.gate = '';
      if (portId === 'control') node.spec.Takeover.control = '';
      setDirty(); render();
    });
  });
}

function connectCable(sourceId, targetNode, portId) {
  if (sourceId === targetNode.id) {
    showNotice('A module cannot patch to itself.');
    return false;
  }
  if (portId === 'signal') targetNode.input = sourceId;
  else if (portId === 'gate' && targetNode.spec.Envelope) targetNode.spec.Envelope.gate = sourceId;
  else if (portId === 'control' && targetNode.spec.Takeover) targetNode.spec.Takeover.control = sourceId;
  else {
    showNotice('That jack cannot accept this cable.');
    return false;
  }
  setDirty();
  clearNotice();
  return true;
}

function markCableTarget(clientX, clientY) {
  clearCableTargets();
  document.elementFromPoint(clientX, clientY)?.closest('.patch-port.input')?.classList.add('cable-target');
}

function clearCableTargets() {
  workspace.querySelectorAll('.cable-target').forEach(port => port.classList.remove('cable-target'));
}

function renderCablePreview(sourceJack, clientX, clientY) {
  renderWires();
  const workspaceRect = workspace.getBoundingClientRect();
  const sourceRect = sourceJack.getBoundingClientRect();
  const x1 = sourceRect.left + sourceRect.width / 2 - workspaceRect.left;
  const y1 = sourceRect.top + sourceRect.height / 2 - workspaceRect.top;
  const x2 = clientX - workspaceRect.left;
  const y2 = clientY - workspaceRect.top;
  const bend = Math.max(50, Math.abs(x2 - x1) * .45);
  const path = document.createElementNS('http://www.w3.org/2000/svg', 'path');
  path.classList.add('preview-wire');
  path.setAttribute('d', `M ${x1} ${y1} C ${x1 + bend} ${y1}, ${x2 - bend} ${y2}, ${x2} ${y2}`);
  wires.appendChild(path);
}

function attachDrag(element, id) {
  const handle = element.querySelector('.node-head');
  handle.addEventListener('pointerdown', event => {
    if (event.button !== 0) return;
    if (event.target.closest('button')) return;
    event.preventDefault();
    const origin = state.positions.get(id);
    const start = { x: event.clientX, y: event.clientY };
    state.pointerInteraction = true;
    element.classList.add('dragging');
    handle.setPointerCapture(event.pointerId);
    const move = moveEvent => {
      const next = {
        x: Math.max(8, Math.min(workspace.clientWidth - element.offsetWidth - 8, origin.x + moveEvent.clientX - start.x)),
        y: Math.max(8, Math.min(workspace.clientHeight - element.offsetHeight - 8, origin.y + moveEvent.clientY - start.y)),
      };
      state.positions.set(id, next);
      element.style.transform = `translate(${next.x}px, ${next.y}px)`;
      renderWires();
    };
    const finish = () => {
      state.pointerInteraction = false;
      element.classList.remove('dragging');
      window.removeEventListener('pointermove', move);
      window.removeEventListener('pointerup', finish);
      window.removeEventListener('pointercancel', finish);
      renderWires();
    };
    window.addEventListener('pointermove', move);
    window.addEventListener('pointerup', finish);
    window.addEventListener('pointercancel', finish);
  });
}

function renderWires() {
  wires.innerHTML = '';
  const workspaceRect = workspace.getBoundingClientRect();
  for (const node of state.graph.nodes) {
    const dependencies = [
      { source: node.input, port: 'signal', dashed: false },
      { source: node.spec.Envelope?.gate, port: 'gate', dashed: true },
      { source: node.spec.Takeover?.control, port: 'control', dashed: true },
    ].filter(dependency => dependency.source);
    for (const dependency of dependencies) {
      const source = workspace.querySelector(`.node[data-id="${CSS.escape(dependency.source)}"]`);
      const target = workspace.querySelector(`.node[data-id="${CSS.escape(node.id)}"]`);
      if (!source || !target) continue;
      const sourceJack = source.querySelector('.patch-port.output .jack');
      const targetJack = target.querySelector(`.patch-port.input .jack[data-port-id="${dependency.port}"]`);
      if (!sourceJack || !targetJack) continue;
      const a = sourceJack.getBoundingClientRect();
      const b = targetJack.getBoundingClientRect();
      const x1 = a.left + a.width / 2 - workspaceRect.left;
      const y1 = a.top + a.height / 2 - workspaceRect.top;
      const x2 = b.left + b.width / 2 - workspaceRect.left;
      const y2 = b.top + b.height / 2 - workspaceRect.top;
      const bend = Math.max(50, Math.abs(x2 - x1) * .45);
      const path = document.createElementNS('http://www.w3.org/2000/svg', 'path');
      path.setAttribute('d', `M ${x1} ${y1} C ${x1 + bend} ${y1}, ${x2 - bend} ${y2}, ${x2} ${y2}`);
      if (dependency.dashed) path.setAttribute('stroke-dasharray', '6 5');
      wires.appendChild(path);
    }
  }
}

function updateMeter(element, value) {
  const numeric = Number(value);
  const normalized = Math.max(0, Math.min(1, Math.abs(numeric) <= 1 ? numeric : numeric / 127));
  element.querySelector('.meter span').style.width = `${normalized * 100}%`;
  element.querySelector('.value').textContent = Number(value).toFixed(3);
}

function renameNode(oldId, newId) {
  if (!newId || state.graph.nodes.some(node => node.id === newId && node.id !== oldId)) {
    showNotice('Node IDs must be non-empty and unique.'); render(); return;
  }
  const node = state.graph.nodes.find(candidate => candidate.id === oldId);
  node.id = newId;
  for (const candidate of state.graph.nodes) if (candidate.input === oldId) candidate.input = newId;
  for (const candidate of state.graph.nodes) if (candidate.spec.Takeover?.control === oldId) candidate.spec.Takeover.control = newId;
  for (const candidate of state.graph.nodes) if (candidate.spec.Envelope?.gate === oldId) candidate.spec.Envelope.gate = newId;
  const position = state.positions.get(oldId);
  state.positions.delete(oldId);
  state.positions.set(newId, position);
  setDirty(); clearNotice(); render();
}

function removeNode(id) {
  state.graph.nodes = state.graph.nodes.filter(node => node.id !== id);
  for (const node of state.graph.nodes) if (node.input === id) node.input = null;
  for (const node of state.graph.nodes) if (node.spec.Takeover?.control === id) node.spec.Takeover.control = '';
  for (const node of state.graph.nodes) if (node.spec.Envelope?.gate === id) node.spec.Envelope.gate = '';
  state.positions.delete(id);
  if (state.pendingCable === id) state.pendingCable = null;
  setDirty(); render();
}

function addModule(module) {
  const id = nextId(module.id);
  state.graph.nodes.push({ id, spec: clone(module.default_spec), input: null });
  setDirty(); clearNotice(); render();
}

function replacementCommands() {
  const commands = state.serverGraph.nodes.map(node => ({ RemoveNode: { id: node.id } }));
  for (const node of state.graph.nodes) commands.push({ AddNode: { id: node.id, spec: clone(node.spec) } });
  for (const node of state.graph.nodes) if (node.input) commands.push({ Connect: { source: node.input, target: node.id } });
  return commands;
}

async function applyPatch() {
  applyButton.disabled = true;
  clearNotice();
  try {
    const response = await fetch('/api/v1/commands', {
      method: 'POST', headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ base_revision: state.revision, commands: replacementCommands() }),
    });
    const body = await response.json();
    if (!response.ok) throw new Error(body.error || `Apply failed (${response.status})`);
    state.revision = body.revision;
    state.serverGraph = clone(state.graph);
    setDirty(false);
    showNotice(body.warning || 'Patch applied at a tick boundary.', body.warning ? 'error' : 'success');
  } catch (error) {
    showNotice(error.message);
    applyButton.disabled = false;
  }
}

function acceptSnapshot(snapshot) {
  state.revision = snapshot.revision;
  state.serverGraph = clone(snapshot.graph);
  if (!state.dirty && !state.pointerInteraction) state.graph = clone(snapshot.graph);
  state.meters = new Map(snapshot.meters.map(meter => [meter.id, meter.value]));
  document.querySelector('#revision').textContent = `REV ${snapshot.revision}`;
  const stats = snapshot.limiter;
  document.querySelector('#runtime-stats').innerHTML = `<dt>Tick</dt><dd>${snapshot.runtime_tick}</dd><dt>Messages</dt><dd>${stats.sent_messages}</dd><dt>Deduped</dt><dd>${stats.deduped}</dd><dt>Bus drops</dt><dd>${stats.bus_rate_limited}</dd>`;
  if (!state.dirty && !state.pointerInteraction) render();
  else workspace.querySelectorAll('.node').forEach(element => updateMeter(element, state.meters.get(element.dataset.id) ?? 0));
}

async function connect() {
  try {
    const [snapshot, catalog] = await Promise.all([
      fetch('/api/v1/state').then(response => response.json()),
      fetch('/api/v1/catalog').then(response => response.json()),
    ]);
    acceptCatalog(catalog);
    acceptSnapshot(snapshot);
    refreshRackSnapshot();
  } catch (error) {
    acceptCatalog({ modules: fallbackModules() });
    showNotice(`Initial state failed: ${error.message}`);
  }
  const protocol = location.protocol === 'https:' ? 'wss' : 'ws';
  const socket = new WebSocket(`${protocol}://${location.host}/api/v1/events`);
  socket.addEventListener('open', () => { const badge = document.querySelector('#connection'); badge.textContent = 'live'; badge.className = 'pill online'; });
  socket.addEventListener('message', event => { const message = JSON.parse(event.data); if (message.type === 'snapshot') acceptSnapshot(message.payload); });
  socket.addEventListener('close', () => { const badge = document.querySelector('#connection'); badge.textContent = 'reconnecting'; badge.className = 'pill offline'; setTimeout(connect, 1200); });
}

moduleSearch.addEventListener('input', renderModuleLibrary);
moduleCategory.addEventListener('change', renderModuleLibrary);
document.querySelector('#reload').addEventListener('click', () => { state.graph = clone(state.serverGraph); setDirty(false); clearNotice(); render(); });
applyButton.addEventListener('click', applyPatch);
copilotAsk.addEventListener('click', proposeRackPatch);
copilotApply.addEventListener('click', applyRackProposal);
copilotUndo.addEventListener('click', undoRackPatch);
document.querySelector('#copilot-reject').addEventListener('click', () => {
  state.rackProposal = null; copilotProposal.hidden = true; clearCopilotNotice();
});
document.querySelectorAll('[data-prompt]').forEach(button => button.addEventListener('click', () => {
  copilotPrompt.value = button.dataset.prompt; copilotPrompt.focus();
}));
window.addEventListener('resize', renderWires);
connect();
