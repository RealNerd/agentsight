// ── Formatting Utilities ─────────────────────────────

function formatTokens(count) {
    if (count == null) return '0';
    return count.toLocaleString('en-US');
}

function formatCost(cost) {
    if (cost == null) return '';
    if (cost < 0.01) return '$' + cost.toFixed(4);
    return '$' + cost.toFixed(2);
}

function formatPercent(ratio) {
    if (ratio == null) return '0.0%';
    return (ratio * 100).toFixed(1) + '%';
}

function formatDate(isoString) {
    if (!isoString) return '—';
    const d = new Date(isoString);
    return d.toLocaleDateString('en-US', { month: 'short', day: 'numeric' });
}

function formatDateTime(isoString) {
    if (!isoString) return '—';
    const d = new Date(isoString);
    return d.toLocaleDateString('en-US', {
        month: 'short', day: 'numeric', year: 'numeric',
        hour: '2-digit', minute: '2-digit'
    });
}

function shortenModel(model) {
    if (!model || model === '<synthetic>') return 'unknown';
    const idx = model.lastIndexOf('-20');
    if (idx > 0 && model.length > 20) return model.substring(0, idx);
    return model;
}

function shortenProject(path) {
    if (!path) return '';
    const parts = path.split('/').filter(Boolean);
    if (parts.length >= 2) return parts[parts.length - 2] + '/' + parts[parts.length - 1];
    return path;
}

// ── API Client ───────────────────────────────────────

const api = {
    async get(path, params = {}) {
        const url = new URL('/api/v1' + path, window.location.origin);
        Object.entries(params).forEach(([k, v]) => {
            if (v != null && v !== '') url.searchParams.set(k, v);
        });
        const res = await fetch(url);
        if (!res.ok) throw new Error(`API ${res.status}: ${res.statusText}`);
        return res.json();
    },

    sessions(params) { return this.get('/sessions', params); },
    session(id) { return this.get('/sessions/' + id); },
    summary(params) { return this.get('/summary', params); },
    timeline(params) { return this.get('/timeline', params); },
    config() { return this.get('/config'); },
    projects() { return this.get('/projects'); },
};

// ── DOM Helpers ──────────────────────────────────────

function el(tag, attrs = {}, children = []) {
    const elem = document.createElement(tag);
    Object.entries(attrs).forEach(([k, v]) => {
        if (k === 'className') elem.className = v;
        else if (k === 'onclick') elem.onclick = v;
        else if (k === 'innerHTML') elem.innerHTML = v;
        else elem.setAttribute(k, v);
    });
    children.forEach(c => {
        if (typeof c === 'string') elem.appendChild(document.createTextNode(c));
        else if (c) elem.appendChild(c);
    });
    return elem;
}

function html(strings, ...values) {
    return strings.reduce((acc, str, i) => acc + str + (values[i] ?? ''), '');
}
