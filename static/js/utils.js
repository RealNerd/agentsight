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

// ── Sortable Table ──────────────────────────────────

/**
 * Create a sortable data table. Clicking headers sorts; clicking again reverses.
 *
 * @param {Object} opts
 * @param {HTMLElement} opts.container - DOM element to render into
 * @param {Array<Object>} opts.columns - Column definitions:
 *   { key, label, align?, sortValue?, format?, headerClass? }
 *   - key: property name on row data
 *   - label: header text
 *   - align: 'right' for numeric columns
 *   - sortValue(row): returns the value to sort by (default: row[key])
 *   - format(row): returns the display HTML string (default: row[key])
 *   - headerClass: extra CSS class for th
 * @param {Array<Object>} opts.rows - The data rows
 * @param {string} [opts.defaultSort] - key to sort by initially
 * @param {boolean} [opts.defaultDesc] - initial sort direction (default true)
 * @param {Function} [opts.onRowClick] - callback(row) when a row is clicked
 * @param {string} [opts.footer] - HTML for footer below table
 */
function sortableTable(opts) {
    const { container, columns, rows, defaultSort, defaultDesc = true, onRowClick, footer } = opts;

    let sortKey = defaultSort || (columns[0] && columns[0].key);
    let sortDesc = defaultDesc;

    function render() {
        const sorted = [...rows].sort((a, b) => {
            const col = columns.find(c => c.key === sortKey);
            const av = col && col.sortValue ? col.sortValue(a) : a[sortKey];
            const bv = col && col.sortValue ? col.sortValue(b) : b[sortKey];
            if (av == null && bv == null) return 0;
            if (av == null) return 1;
            if (bv == null) return -1;
            let cmp;
            if (typeof av === 'string') cmp = av.localeCompare(bv);
            else cmp = av < bv ? -1 : av > bv ? 1 : 0;
            return sortDesc ? -cmp : cmp;
        });

        const ths = columns.map(col => {
            const arrow = col.key === sortKey ? (sortDesc ? ' ▾' : ' ▴') : '';
            const cls = [col.align === 'right' ? 'right' : '', 'sortable-th', col.headerClass || ''].filter(Boolean).join(' ');
            return `<th class="${cls}" data-sort-key="${col.key}">${col.label}${arrow}</th>`;
        }).join('');

        const trs = sorted.map(row => {
            const tds = columns.map(col => {
                const val = col.format ? col.format(row) : (row[col.key] ?? '');
                const cls = col.align === 'right' ? 'right mono' : '';
                return `<td class="${cls}">${val}</td>`;
            }).join('');
            const clickAttr = onRowClick ? `class="clickable-row"` : '';
            return `<tr ${clickAttr} data-row-idx="${rows.indexOf(row)}">${tds}</tr>`;
        }).join('');

        container.innerHTML = `
            <table class="data-table">
                <thead><tr>${ths}</tr></thead>
                <tbody>${trs}</tbody>
            </table>
            ${footer ? `<div style="color: var(--text-muted); font-size: 0.8rem;">${footer}</div>` : ''}
        `;

        // Bind header clicks
        container.querySelectorAll('.sortable-th').forEach(th => {
            th.addEventListener('click', () => {
                const key = th.dataset.sortKey;
                if (sortKey === key) {
                    sortDesc = !sortDesc;
                } else {
                    sortKey = key;
                    sortDesc = true;
                }
                render();
            });
        });

        // Bind row clicks
        if (onRowClick) {
            container.querySelectorAll('.clickable-row').forEach(tr => {
                tr.addEventListener('click', () => {
                    const idx = parseInt(tr.dataset.rowIdx, 10);
                    onRowClick(rows[idx]);
                });
            });
        }
    }

    render();
}
