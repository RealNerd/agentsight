// ── Router ────────────────────────────────────────────
let appConfig = null;

async function loadConfig() {
    if (!appConfig) {
        appConfig = await api.config();
    }
    return appConfig;
}

function route() {
    const hash = window.location.hash || '#/';
    const path = hash.slice(1); // remove #

    // Update active nav link
    document.querySelectorAll('.nav-link').forEach(link => {
        const href = link.getAttribute('href');
        link.classList.toggle('active', href === '#' + path || (path === '/' && href === '#/'));
    });

    if (path === '/' || path === '') {
        renderOverview();
    } else if (path === '/sessions') {
        renderSessionsList();
    } else if (path.startsWith('/session/')) {
        const id = path.replace('/session/', '');
        renderSessionDetail(id);
    } else if (path === '/summary') {
        renderSummaryPage();
    } else if (path === '/timeline') {
        renderTimelinePage();
    } else if (path === '/watch') {
        renderWatchPage();
    } else {
        document.getElementById('app').innerHTML = '<div class="empty-state">Page not found</div>';
    }
}

window.addEventListener('hashchange', route);
window.addEventListener('DOMContentLoaded', route);

// ── Overview Page ─────────────────────────────────────
async function renderOverview() {
    const app = document.getElementById('app');
    app.innerHTML = '<div class="loading">Loading overview...</div>';

    try {
        const [summary, config] = await Promise.all([
            api.summary({ days: 7 }),
            loadConfig(),
        ]);

        const todaySummary = await api.summary({ days: 1 });

        app.innerHTML = `
            <h1 class="page-title">Overview</h1>

            <div class="kpi-grid">
                <div class="kpi-card">
                    <div class="kpi-label">Today's Tokens</div>
                    <div class="kpi-value accent">${formatTokens(todaySummary.total_tokens)}</div>
                </div>
                <div class="kpi-card">
                    <div class="kpi-label">7-Day Tokens</div>
                    <div class="kpi-value">${formatTokens(summary.total_tokens)}</div>
                </div>
                <div class="kpi-card">
                    <div class="kpi-label">Cache Hit Ratio</div>
                    <div class="kpi-value green">${formatPercent(summary.cache_hit_ratio)}</div>
                </div>
                <div class="kpi-card">
                    <div class="kpi-label">Sessions (7d)</div>
                    <div class="kpi-value">${summary.session_count}</div>
                </div>
                ${config.show_cost ? `
                <div class="kpi-card">
                    <div class="kpi-label">7-Day Cost</div>
                    <div class="kpi-value">${formatCost(summary.total_cost)}</div>
                </div>` : ''}
            </div>

            <div class="chart-grid">
                <div class="chart-card">
                    <div class="chart-title">Daily Token Trend (7 days)</div>
                    <div class="chart-container"><canvas id="chart-daily"></canvas></div>
                </div>
                <div class="chart-card">
                    <div class="chart-title">Tokens by Project</div>
                    <div class="chart-container"><canvas id="chart-projects"></canvas></div>
                </div>
            </div>

            <div id="insights-container"></div>
        `;

        // Daily bar chart
        if (summary.by_day && summary.by_day.length > 0) {
            const days = summary.by_day;
            createBarChart('chart-daily',
                days.map(d => d.date.slice(5)), // MM-DD
                [{
                    label: 'Tokens',
                    data: days.map(d => d.tokens),
                    backgroundColor: COLORS.primary + '99',
                    borderColor: COLORS.primary,
                    borderWidth: 1,
                }]
            );
        }

        // Project doughnut
        if (summary.by_project && summary.by_project.length > 0) {
            createDoughnutChart('chart-projects',
                summary.by_project.map(p => p.project),
                summary.by_project.map(p => p.tokens)
            );
        }

        // Insights
        renderInsights(summary, todaySummary);

    } catch (err) {
        app.innerHTML = `<div class="empty-state">Error loading overview: ${err.message}</div>`;
    }
}

function renderInsights(summary, todaySummary) {
    const container = document.getElementById('insights-container');
    if (!container) return;

    const insights = [];

    // Low cache hit sessions
    if (summary.cache_hit_ratio < 0.8) {
        insights.push({
            icon: 'warn',
            text: `Overall cache hit ratio is ${formatPercent(summary.cache_hit_ratio)} — below 80%. Consider reducing context churn between turns.`
        });
    }

    // Dominant project
    if (summary.by_project) {
        const dominant = summary.by_project.find(p => p.pct > 50);
        if (dominant) {
            insights.push({
                icon: 'info',
                text: `${dominant.project} accounts for ${dominant.pct.toFixed(1)}% of all tokens.`
            });
        }
    }

    // High output ratio
    if (summary.total_tokens > 0 && todaySummary.total_tokens > 0) {
        const ratio = todaySummary.total_tokens / (summary.total_tokens / 7);
        if (ratio > 2) {
            insights.push({
                icon: 'warn',
                text: `Today's usage is ${ratio.toFixed(1)}x the daily average. Unusually high activity.`
            });
        }
    }

    if (summary.cache_hit_ratio >= 0.9) {
        insights.push({
            icon: 'good',
            text: `Cache hit ratio of ${formatPercent(summary.cache_hit_ratio)} is excellent. Context reuse is working well.`
        });
    }

    if (insights.length === 0) {
        insights.push({ icon: 'good', text: 'No actionable insights right now. Usage looks healthy.' });
    }

    const iconMap = { warn: '!', info: 'i', good: '*' };
    container.innerHTML = `
        <div class="insights-panel">
            <div class="insights-title">Insights</div>
            ${insights.map(i => `
                <div class="insight-item">
                    <span class="insight-icon ${i.icon}">[${iconMap[i.icon]}]</span>
                    <span>${i.text}</span>
                </div>
            `).join('')}
        </div>
    `;
}

// ── Sessions List Page ────────────────────────────────
async function renderSessionsList() {
    const app = document.getElementById('app');
    app.innerHTML = '<div class="loading">Loading sessions...</div>';

    try {
        const [projects, config] = await Promise.all([
            api.projects(),
            loadConfig(),
        ]);

        // Render shell with filters, then load data
        app.innerHTML = `
            <h1 class="page-title">Sessions</h1>
            <div class="filter-bar">
                <label>Days</label>
                <select id="filter-days">
                    <option value="1">1</option>
                    <option value="3">3</option>
                    <option value="7" selected>7</option>
                    <option value="14">14</option>
                    <option value="30">30</option>
                </select>
                <label>Project</label>
                <select id="filter-project">
                    <option value="">All</option>
                    ${projects.map(p => `<option value="${p}">${p}</option>`).join('')}
                </select>
            </div>
            <div id="sessions-table"><div class="loading">Loading...</div></div>
        `;

        async function loadSessions() {
            const days = document.getElementById('filter-days').value;
            const project = document.getElementById('filter-project').value;

            const data = await api.sessions({ days, project, limit: 100 });
            renderSessionsTable(data, config.show_cost);
        }

        document.getElementById('filter-days').addEventListener('change', loadSessions);
        document.getElementById('filter-project').addEventListener('change', loadSessions);

        await loadSessions();

    } catch (err) {
        app.innerHTML = `<div class="empty-state">Error: ${err.message}</div>`;
    }
}

function renderSessionsTable(data, showCost) {
    const container = document.getElementById('sessions-table');
    if (!container) return;

    if (data.sessions.length === 0) {
        container.innerHTML = '<div class="empty-state">No sessions found for this period.</div>';
        return;
    }

    // Pre-compute display labels for slug disambiguation
    const sessions = data.sessions.map(s => {
        const base = s.slug || s.session_id.slice(0, 8);
        const hasDupe = s.slug && data.sessions.some(
            o => o.slug === s.slug && o.session_id !== s.session_id
        );
        return { ...s, _label: hasDupe ? base + ' (' + s.session_id.slice(0, 4) + ')' : base };
    });

    const columns = [
        { key: 'session', label: 'Session', sortValue: r => (r._label || '').toLowerCase(), format: r => r._label },
        { key: 'project', label: 'Project', sortValue: r => shortenProject(r.project).toLowerCase(), format: r => shortenProject(r.project) },
        { key: 'date', label: 'Date', sortValue: r => r.start_time || '', format: r => formatDate(r.start_time) },
        { key: 'model', label: 'Model', sortValue: r => (r.model || '').toLowerCase(), format: r => shortenModel(r.model) },
        { key: 'tokens', label: 'Tokens', align: 'right', sortValue: r => r.tokens.total, format: r => formatTokens(r.tokens.total) },
    ];
    if (showCost) {
        columns.push({ key: 'cost', label: 'Cost', align: 'right', sortValue: r => r.cost ? r.cost.total : 0, format: r => r.cost ? formatCost(r.cost.total) : '' });
    }
    columns.push(
        { key: 'cache', label: 'Cache', align: 'right', sortValue: r => r.cache_hit_ratio, format: r => formatPercent(r.cache_hit_ratio) },
        { key: 'turns', label: 'Turns', align: 'right', sortValue: r => r.turns, format: r => r.turns },
    );

    const footer = `${data.session_count} sessions, ${formatTokens(data.total_tokens)} total tokens`
        + (data.total_cost != null ? ', ' + formatCost(data.total_cost) + ' total cost' : '');

    sortableTable({
        container,
        columns,
        rows: sessions,
        defaultSort: 'date',
        defaultDesc: true,
        onRowClick: r => { window.location.hash = '#/session/' + r.session_id; },
        footer,
    });
}

// ── Session Detail Page ───────────────────────────────
async function renderSessionDetail(id) {
    const app = document.getElementById('app');
    app.innerHTML = '<div class="loading">Loading session...</div>';

    try {
        const [data, config] = await Promise.all([
            api.session(id),
            loadConfig(),
        ]);

        const s = data;
        const showCost = config.show_cost;
        const totalTokens = s.tokens.total;

        app.innerHTML = `
            <a href="#/sessions" class="back-link">&larr; Back to sessions</a>

            <div class="session-header">
                <h2 style="margin-bottom: 0.75rem;">${s.slug || s.session_id.slice(0, 8)}</h2>
                <div class="session-meta">
                    <div class="meta-item">
                        <div class="meta-label">Session ID</div>
                        <div>${s.session_id}</div>
                    </div>
                    <div class="meta-item">
                        <div class="meta-label">Project</div>
                        <div>${s.project}</div>
                    </div>
                    <div class="meta-item">
                        <div class="meta-label">Date</div>
                        <div>${formatDateTime(s.start_time)} &mdash; ${s.end_time ? new Date(s.end_time).toLocaleTimeString() : ''}</div>
                    </div>
                    <div class="meta-item">
                        <div class="meta-label">Model</div>
                        <div>${s.model || 'unknown'}</div>
                    </div>
                    ${s.git_branch ? `<div class="meta-item"><div class="meta-label">Branch</div><div>${s.git_branch}</div></div>` : ''}
                    <div class="meta-item">
                        <div class="meta-label">Turns</div>
                        <div>${s.turns}</div>
                    </div>
                </div>
            </div>

            <div class="chart-grid">
                <div class="chart-card">
                    <div class="chart-title">Token Composition</div>
                    <div class="chart-container"><canvas id="chart-composition"></canvas></div>
                </div>
                <div class="chart-card">
                    <div class="chart-title">Cumulative Tokens per Turn</div>
                    <div class="chart-container"><canvas id="chart-cumulative"></canvas></div>
                </div>
            </div>

            <h3 class="page-title" style="margin-top: 1rem;">Token Breakdown</h3>
            <table class="data-table">
                <thead>
                    <tr>
                        <th>Category</th>
                        <th class="right">Tokens</th>
                        <th class="right">% of Total</th>
                        ${showCost ? '<th class="right">Cost</th>' : ''}
                    </tr>
                </thead>
                <tbody>
                    ${[
                        ['Input', s.tokens.input, s.cost?.input],
                        ['Cache Creation', s.tokens.cache_creation, s.cost?.cache_creation],
                        ['Cache Read', s.tokens.cache_read, s.cost?.cache_read],
                        ['Output', s.tokens.output, s.cost?.output],
                    ].map(([label, tokens, cost]) => `
                        <tr>
                            <td>${label}</td>
                            <td class="right mono">${formatTokens(tokens)}</td>
                            <td class="right mono">${totalTokens > 0 ? ((tokens / totalTokens) * 100).toFixed(1) + '%' : '0%'}</td>
                            ${showCost ? `<td class="right mono">${cost != null ? formatCost(cost) : ''}</td>` : ''}
                        </tr>
                    `).join('')}
                    <tr style="font-weight: 600;">
                        <td>Total</td>
                        <td class="right mono">${formatTokens(totalTokens)}</td>
                        <td class="right mono">100.0%</td>
                        ${showCost ? `<td class="right mono">${s.cost ? formatCost(s.cost.total) : ''}</td>` : ''}
                    </tr>
                </tbody>
            </table>

            ${Object.keys(s.tool_calls || {}).length > 0 ? `
            <h3 class="page-title">Tool Usage</h3>
            <div class="chart-grid">
                <div class="chart-card" style="grid-column: 1 / -1;">
                    <div class="chart-container" style="height: 200px;"><canvas id="chart-tools"></canvas></div>
                </div>
            </div>
            ` : ''}

            ${s.turn_details && s.turn_details.length > 0 ? `
            <h3 class="page-title">Turn Details</h3>
            <div id="turn-details-table"></div>
            ` : ''}
        `;

        // Token composition doughnut
        createDoughnutChart('chart-composition',
            ['Input', 'Cache Creation', 'Cache Read', 'Output'],
            [s.tokens.input, s.tokens.cache_creation, s.tokens.cache_read, s.tokens.output],
            [COLORS.primary, COLORS.yellow, COLORS.green, COLORS.purple]
        );

        // Cumulative token line chart
        if (s.turn_details && s.turn_details.length > 0) {
            let cumInput = 0, cumCacheRead = 0, cumOutput = 0;
            const labels = [];
            const inputData = [], cacheData = [], outputData = [];

            s.turn_details.forEach(t => {
                cumInput += t.tokens.input + t.tokens.cache_creation;
                cumCacheRead += t.tokens.cache_read;
                cumOutput += t.tokens.output;
                labels.push(String(t.index));
                inputData.push(cumInput);
                cacheData.push(cumCacheRead);
                outputData.push(cumOutput);
            });

            createLineChart('chart-cumulative', labels, [
                { label: 'Input + Cache Write', data: inputData, borderColor: COLORS.primary, backgroundColor: COLORS.primary + '20' },
                { label: 'Cache Read', data: cacheData, borderColor: COLORS.green, backgroundColor: COLORS.green + '20' },
                { label: 'Output', data: outputData, borderColor: COLORS.purple, backgroundColor: COLORS.purple + '20' },
            ]);
        }

        // Turn details sortable table
        const turnContainer = document.getElementById('turn-details-table');
        if (turnContainer && s.turn_details && s.turn_details.length > 0) {
            sortableTable({
                container: turnContainer,
                columns: [
                    { key: 'index', label: '#', sortValue: r => r.index, format: r => r.index },
                    { key: 'time', label: 'Time', sortValue: r => r.timestamp || '', format: r => r.timestamp ? new Date(r.timestamp).toLocaleTimeString() : '' },
                    { key: 'input', label: 'Input', align: 'right', sortValue: r => r.tokens.input, format: r => formatTokens(r.tokens.input) },
                    { key: 'cache_read', label: 'Cache Read', align: 'right', sortValue: r => r.tokens.cache_read, format: r => formatTokens(r.tokens.cache_read) },
                    { key: 'output', label: 'Output', align: 'right', sortValue: r => r.tokens.output, format: r => formatTokens(r.tokens.output) },
                    { key: 'total', label: 'Total', align: 'right', sortValue: r => r.tokens.total, format: r => formatTokens(r.tokens.total) },
                    { key: 'tools', label: 'Tools', sortValue: r => r.tools.join(', '), format: r => r.tools.join(', ') || '—' },
                ],
                rows: s.turn_details,
                defaultSort: 'index',
                defaultDesc: false,
            });
        }

        // Tool usage bar chart
        if (s.tool_calls && Object.keys(s.tool_calls).length > 0) {
            const sorted = Object.entries(s.tool_calls).sort((a, b) => b[1] - a[1]);
            createBarChart('chart-tools',
                sorted.map(([name]) => name),
                [{
                    label: 'Calls',
                    data: sorted.map(([, count]) => count),
                    backgroundColor: COLORS.primary + '99',
                    borderColor: COLORS.primary,
                    borderWidth: 1,
                }],
                { formatValue: v => v + ' calls' }
            );
        }

    } catch (err) {
        app.innerHTML = `<div class="empty-state">Error loading session: ${err.message}</div>`;
    }
}

// ── Summary Page ──────────────────────────────────────
async function renderSummaryPage() {
    const app = document.getElementById('app');
    app.innerHTML = '<div class="loading">Loading summary...</div>';

    try {
        const [projects, config] = await Promise.all([
            api.projects(),
            loadConfig(),
        ]);

        app.innerHTML = `
            <h1 class="page-title">Summary</h1>
            <div class="filter-bar">
                <label>Period</label>
                <select id="summary-days">
                    <option value="7" selected>7 days</option>
                    <option value="14">14 days</option>
                    <option value="30">30 days</option>
                </select>
                <label>Project</label>
                <select id="summary-project">
                    <option value="">All</option>
                    ${projects.map(p => `<option value="${p}">${p}</option>`).join('')}
                </select>
            </div>
            <div id="summary-content"><div class="loading">Loading...</div></div>
        `;

        async function loadSummary() {
            const days = document.getElementById('summary-days').value;
            const project = document.getElementById('summary-project').value;
            const data = await api.summary({ days, project });
            renderSummaryContent(data, config.show_cost);
        }

        document.getElementById('summary-days').addEventListener('change', loadSummary);
        document.getElementById('summary-project').addEventListener('change', loadSummary);

        await loadSummary();

    } catch (err) {
        app.innerHTML = `<div class="empty-state">Error: ${err.message}</div>`;
    }
}

function renderSummaryContent(data, showCost) {
    const container = document.getElementById('summary-content');
    if (!container) return;

    container.innerHTML = `
        <div class="kpi-grid">
            <div class="kpi-card">
                <div class="kpi-label">Sessions</div>
                <div class="kpi-value">${data.session_count}</div>
            </div>
            <div class="kpi-card">
                <div class="kpi-label">Total Tokens</div>
                <div class="kpi-value accent">${formatTokens(data.total_tokens)}</div>
            </div>
            <div class="kpi-card">
                <div class="kpi-label">Avg Tokens/Session</div>
                <div class="kpi-value">${formatTokens(data.avg_tokens_per_session)}</div>
            </div>
            <div class="kpi-card">
                <div class="kpi-label">Cache Hit Ratio</div>
                <div class="kpi-value green">${formatPercent(data.cache_hit_ratio)}</div>
            </div>
            <div class="kpi-card">
                <div class="kpi-label">Avg Tokens/Hour</div>
                <div class="kpi-value">${formatTokens(data.avg_tokens_per_hour)}</div>
            </div>
            ${data.peak_hour ? `
            <div class="kpi-card">
                <div class="kpi-label">Peak Hour</div>
                <div class="kpi-value yellow">${formatTokens(data.peak_hour.tokens)}</div>
                <div class="kpi-sub">${data.peak_hour.hour}</div>
            </div>` : ''}
            ${showCost && data.total_cost != null ? `
            <div class="kpi-card">
                <div class="kpi-label">Total Cost</div>
                <div class="kpi-value">${formatCost(data.total_cost)}</div>
            </div>` : ''}
        </div>

        <div class="chart-grid">
            <div class="chart-card">
                <div class="chart-title">Daily Token Trend</div>
                <div class="chart-container"><canvas id="summary-daily"></canvas></div>
            </div>
            <div class="chart-card">
                <div class="chart-title">By Project</div>
                <div class="chart-container"><canvas id="summary-projects"></canvas></div>
            </div>
        </div>

        ${data.by_project.length > 0 ? '<h3 class="page-title">By Project</h3><div id="summary-by-project"></div>' : ''}
        ${data.by_model.length > 0 ? '<h3 class="page-title">By Model</h3><div id="summary-by-model"></div>' : ''}
    `;

    // Daily trend chart
    if (data.by_day && data.by_day.length > 0) {
        createBarChart('summary-daily',
            data.by_day.map(d => d.date.slice(5)),
            [{
                label: 'Tokens',
                data: data.by_day.map(d => d.tokens),
                backgroundColor: COLORS.primary + '99',
                borderColor: COLORS.primary,
                borderWidth: 1,
            }]
        );
    }

    // Project doughnut
    if (data.by_project && data.by_project.length > 0) {
        createDoughnutChart('summary-projects',
            data.by_project.map(p => p.project),
            data.by_project.map(p => p.tokens)
        );
    }

    // By Project sortable table
    const projContainer = document.getElementById('summary-by-project');
    if (projContainer && data.by_project.length > 0) {
        const projCols = [
            { key: 'project', label: 'Project', sortValue: r => r.project.toLowerCase(), format: r => r.project },
            { key: 'tokens', label: 'Tokens', align: 'right', sortValue: r => r.tokens, format: r => formatTokens(r.tokens) },
            { key: 'sessions', label: 'Sessions', align: 'right', sortValue: r => r.sessions, format: r => r.sessions },
            { key: 'pct', label: '%', align: 'right', sortValue: r => r.pct, format: r => r.pct.toFixed(1) + '%' },
        ];
        if (showCost) {
            projCols.push({ key: 'cost', label: 'Cost', align: 'right', sortValue: r => r.cost || 0, format: r => r.cost != null ? formatCost(r.cost) : '' });
        }
        sortableTable({ container: projContainer, columns: projCols, rows: data.by_project, defaultSort: 'tokens', defaultDesc: true });
    }

    // By Model sortable table
    const modelContainer = document.getElementById('summary-by-model');
    if (modelContainer && data.by_model.length > 0) {
        const modelCols = [
            { key: 'model', label: 'Model', sortValue: r => r.model.toLowerCase(), format: r => r.model },
            { key: 'tokens', label: 'Tokens', align: 'right', sortValue: r => r.tokens, format: r => formatTokens(r.tokens) },
            { key: 'pct', label: '%', align: 'right', sortValue: r => r.pct, format: r => r.pct.toFixed(1) + '%' },
        ];
        if (showCost) {
            modelCols.push({ key: 'cost', label: 'Cost', align: 'right', sortValue: r => r.cost || 0, format: r => r.cost != null ? formatCost(r.cost) : '' });
        }
        sortableTable({ container: modelContainer, columns: modelCols, rows: data.by_model, defaultSort: 'tokens', defaultDesc: true });
    }
}

// ── Timeline Page ─────────────────────────────────────
async function renderTimelinePage() {
    const app = document.getElementById('app');
    app.innerHTML = '<div class="loading">Loading timeline...</div>';
    destroyAllCharts();

    try {
        const [projects, config] = await Promise.all([
            api.projects(),
            loadConfig(),
        ]);

        app.innerHTML = `
            <h1 class="page-title">Timeline</h1>
            <div class="filter-bar">
                <label>Days</label>
                <select id="tl-days">
                    <option value="1" selected>1</option>
                    <option value="3">3</option>
                    <option value="7">7</option>
                    <option value="14">14</option>
                    <option value="30">30</option>
                </select>
                <label>Project</label>
                <select id="tl-project">
                    <option value="">All</option>
                    ${projects.map(p => `<option value="${p}">${p}</option>`).join('')}
                </select>
                <span id="tl-peak" style="margin-left: auto; color: var(--text-muted); font-size: 0.85rem;"></span>
            </div>
            <div id="tl-content"><div class="loading">Loading...</div></div>
        `;

        async function loadTimeline() {
            const days = document.getElementById('tl-days').value;
            const project = document.getElementById('tl-project').value;
            const data = await api.timeline({ days, project });
            renderTimelineContent(data, config.show_cost);
        }

        document.getElementById('tl-days').addEventListener('change', loadTimeline);
        document.getElementById('tl-project').addEventListener('change', loadTimeline);

        await loadTimeline();

    } catch (err) {
        app.innerHTML = `<div class="empty-state">Error: ${err.message}</div>`;
    }
}

function renderTimelineContent(data, showCost) {
    const container = document.getElementById('tl-content');
    if (!container) return;
    destroyAllCharts();

    // Update peak indicator
    const peakEl = document.getElementById('tl-peak');
    if (peakEl) {
        peakEl.textContent = data.total_sessions > 0
            ? `Peak: ${data.peak_concurrent} concurrent | ${data.total_sessions} sessions`
            : '';
    }

    if (data.total_sessions === 0) {
        container.innerHTML = '<div class="empty-state">No sessions with time data found for this period.</div>';
        return;
    }

    // Build project color map
    const projectSet = [...new Set(data.sessions.map(s => s.project))];
    const projectColors = {};
    projectSet.forEach((p, i) => {
        projectColors[p] = PALETTE[i % PALETTE.length];
    });

    container.innerHTML = `
        <div class="chart-grid">
            <div class="chart-card" style="grid-column: 1 / -1;">
                <div class="chart-title">Concurrency Over Time</div>
                <div class="chart-container"><canvas id="tl-concurrency"></canvas></div>
            </div>
        </div>

        <div class="chart-card" style="margin-bottom: 1.5rem;">
            <div class="chart-title">Session Gantt</div>
            <div id="tl-gantt" class="gantt-container"></div>
        </div>

        <div class="chart-grid">
            <div class="chart-card" style="grid-column: 1 / -1;">
                <div class="chart-title">Token Burn Rate</div>
                <div class="chart-container"><canvas id="tl-burn"></canvas></div>
            </div>
        </div>

        <div style="color: var(--text-muted); font-size: 0.8rem; margin-top: 0.5rem;">
            ${data.total_sessions} sessions across ${data.period_days} day(s)
        </div>
    `;

    // 1. Concurrency area chart
    if (data.concurrency.length > 0) {
        const labels = data.concurrency.map(c => formatTimeLabel(c.time, data.period_days));
        createLineChart('tl-concurrency', labels, [{
            label: 'Concurrent Sessions',
            data: data.concurrency.map(c => c.count),
            borderColor: COLORS.primary,
            backgroundColor: COLORS.primary + '30',
            fill: true,
        }], {
            formatValue: v => v + ' sessions',
            yTickFormat: v => v,
            chartOptions: {
                scales: {
                    y: {
                        beginAtZero: true,
                        ticks: {
                            stepSize: 1,
                            callback: v => Number.isInteger(v) ? v : '',
                        }
                    }
                }
            }
        });
    }

    // 2. Gantt chart (HTML/CSS positioned divs)
    renderGanttChart(data, projectColors, showCost);

    // 3. Token burn rate bar chart (stacked by project)
    if (data.concurrency.length > 0) {
        const labels = data.concurrency.map(c => formatTimeLabel(c.time, data.period_days));

        // For a simple non-stacked view, just use total tokens per slot
        createBarChart('tl-burn', labels, [{
            label: 'Tokens',
            data: data.concurrency.map(c => c.tokens),
            backgroundColor: COLORS.primary + '99',
            borderColor: COLORS.primary,
            borderWidth: 1,
        }]);
    }
}

function renderGanttChart(data, projectColors, showCost) {
    const ganttEl = document.getElementById('tl-gantt');
    if (!ganttEl || data.sessions.length === 0) return;

    const periodStart = new Date(data.period_start).getTime();
    const periodEnd = new Date(data.period_end).getTime();
    const totalMs = periodEnd - periodStart;
    if (totalMs <= 0) return;

    // Group by project
    const projectOrder = [];
    const byProject = {};
    data.sessions.forEach(s => {
        if (!byProject[s.project]) {
            byProject[s.project] = [];
            projectOrder.push(s.project);
        }
        byProject[s.project].push(s);
    });

    let html = '';

    // Time axis ticks
    const numTicks = Math.min(data.concurrency.length, 12);
    const tickStep = Math.max(1, Math.floor(data.concurrency.length / numTicks));
    html += '<div class="gantt-axis">';
    for (let i = 0; i < data.concurrency.length; i += tickStep) {
        const c = data.concurrency[i];
        const t = new Date(c.time).getTime();
        const pct = ((t - periodStart) / totalMs * 100);
        html += `<span class="gantt-tick" style="left:${pct}%">${formatTimeLabel(c.time, data.period_days)}</span>`;
    }
    html += '</div>';

    // Session bars
    for (const project of projectOrder) {
        html += `<div class="gantt-project-label">${project}</div>`;

        for (const s of byProject[project]) {
            const startMs = new Date(s.start_time).getTime();
            const endMs = new Date(s.end_time).getTime();
            const left = ((startMs - periodStart) / totalMs * 100);
            const width = Math.max(0.5, (endMs - startMs) / totalMs * 100);
            const color = projectColors[s.project] || COLORS.primary;

            const label = s.slug || s.session_id.slice(0, 8);
            const durationStr = s.duration_minutes + 'm';
            const tokenStr = formatTokens(s.tokens);
            const cacheStr = formatPercent(s.cache_hit_ratio);
            const costStr = showCost && s.cost != null ? ' | ' + formatCost(s.cost) : '';
            const tooltip = `${label} — ${tokenStr} tokens, ${durationStr}, cache ${cacheStr}${costStr}`;

            html += `<div class="gantt-row">
                <div class="gantt-bar" style="left:${left}%;width:${width}%;background:${color}"
                     title="${tooltip}"
                     onclick="window.location.hash='#/session/${s.session_id}'">
                    ${width > 8 ? `<span class="gantt-bar-label">${label}</span>` : ''}
                </div>
            </div>`;
        }
    }

    ganttEl.innerHTML = html;
}

function formatTimeLabel(isoString, periodDays) {
    const d = new Date(isoString);
    if (periodDays <= 3) {
        return d.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
    }
    return d.toLocaleDateString('en-US', { month: 'short', day: 'numeric' }) +
           ' ' + d.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' });
}

// ── Watch Page ────────────────────────────────────────
let watchEventSource = null;
let watchTickInterval = null;
let watchPageVisible = true;

function cleanupWatch() {
    if (watchEventSource) {
        watchEventSource.close();
        watchEventSource = null;
    }
    if (watchTickInterval) {
        clearInterval(watchTickInterval);
        watchTickInterval = null;
    }
}

async function renderWatchPage() {
    cleanupWatch();
    destroyAllCharts();

    const app = document.getElementById('app');
    const config = await loadConfig();

    app.innerHTML = `
        <h1 class="page-title">Live Watch</h1>
        <div class="watch-status">
            <span class="status-dot" id="watch-dot"></span>
            <span id="watch-status-text">Connecting...</span>
        </div>
        <div id="watch-table"><div class="empty-state">Waiting for active sessions...</div></div>
        <div class="chart-grid" style="margin-top: 1rem;">
            <div class="chart-card" style="grid-column: 1 / -1;">
                <div class="chart-title">Token Burn Rate (rolling)</div>
                <div class="chart-container"><canvas id="chart-burn"></canvas></div>
            </div>
        </div>
    `;

    const burnLabels = [];
    const burnDeltas = [];
    let lastTotalTokens = null;
    let pendingDelta = 0;
    let burnChart = null;
    const MAX_BURN_POINTS = 60;
    const TICK_INTERVAL_MS = 2000;
    let lastRenderedSnapshotKey = '';

    // Push a data point every tick, whether or not an SSE event arrived
    function pushBurnPoint() {
        // Skip chart updates when tab is hidden to avoid wasted CPU/memory
        if (document.hidden) return;

        const now = new Date().toLocaleTimeString([], { hour: '2-digit', minute: '2-digit', second: '2-digit' });
        burnLabels.push(now);
        burnDeltas.push(pendingDelta);
        pendingDelta = 0; // consumed

        if (burnLabels.length > MAX_BURN_POINTS) {
            burnLabels.shift();
            burnDeltas.shift();
        }

        if (!burnChart) {
            // Create chart once
            burnChart = createLineChart('chart-burn', burnLabels, [{
                label: 'Tokens/interval',
                data: burnDeltas,
                borderColor: COLORS.primary,
                backgroundColor: COLORS.primary + '20',
                fill: true,
            }], {
                formatValue: v => formatTokens(v) + ' tokens',
                chartOptions: {
                    animation: false,
                },
            });
        } else {
            // Update in-place — no destroy/recreate
            burnChart.data.labels = burnLabels;
            burnChart.data.datasets[0].data = burnDeltas;
            burnChart.update('none');
        }
    }

    watchTickInterval = setInterval(pushBurnPoint, TICK_INTERVAL_MS);

    watchEventSource = new EventSource('/api/v1/watch/stream');

    watchEventSource.addEventListener('snapshot', (event) => {
        try {
            const snapshot = JSON.parse(event.data);

            // Always accumulate delta regardless of visibility
            const currentTotal = snapshot.total_tokens;
            if (lastTotalTokens !== null) {
                pendingDelta += Math.max(0, currentTotal - lastTotalTokens);
            }
            lastTotalTokens = currentTotal;

            // Skip DOM updates when tab is hidden
            if (document.hidden) return;

            // Update status
            document.getElementById('watch-dot').className = 'status-dot';
            document.getElementById('watch-status-text').textContent =
                `Connected — ${snapshot.active_sessions.length} active session(s)`;

            // Skip table re-render if data hasn't changed
            const snapshotKey = snapshot.total_tokens + ':' + snapshot.active_sessions.length;
            if (snapshotKey === lastRenderedSnapshotKey) return;
            lastRenderedSnapshotKey = snapshotKey;

            renderWatchTable(snapshot, config.show_cost);
        } catch (e) {
            // Ignore parse errors
        }
    });

    watchEventSource.onerror = () => {
        const dot = document.getElementById('watch-dot');
        const text = document.getElementById('watch-status-text');
        if (dot) dot.className = 'status-dot disconnected';
        if (text) text.textContent = 'Disconnected — retrying...';
    };

    watchEventSource.onopen = () => {
        const dot = document.getElementById('watch-dot');
        const text = document.getElementById('watch-status-text');
        if (dot) dot.className = 'status-dot';
        if (text) text.textContent = 'Connected — waiting for updates...';
    };
}

function watchSessionLabel(s, allSessions) {
    const base = s.slug || s.session_id.slice(0, 8);
    const hasDupe = s.slug && allSessions.some(
        other => other.slug === s.slug && other.session_id !== s.session_id
    );
    return hasDupe ? base + ' (' + s.session_id.slice(0, 4) + ')' : base;
}

function renderWatchTable(snapshot, showCost) {
    const container = document.getElementById('watch-table');
    if (!container) return;

    if (snapshot.active_sessions.length === 0) {
        container.innerHTML = '<div class="empty-state">No active sessions</div>';
        return;
    }

    const sessions = snapshot.active_sessions;

    container.innerHTML = `
        <table class="data-table">
            <thead>
                <tr>
                    <th>Session</th>
                    <th>Project</th>
                    <th>Started</th>
                    <th>Model</th>
                    <th class="right">Tokens</th>
                    ${showCost ? '<th class="right">Cost</th>' : ''}
                    <th class="right">Cache</th>
                    <th class="right">Turns</th>
                </tr>
            </thead>
            <tbody>
                ${sessions.map(s => {
                    const label = watchSessionLabel(s, sessions);
                    const started = s.start_time
                        ? new Date(s.start_time).toLocaleTimeString([], {hour:'2-digit', minute:'2-digit'})
                        : '';
                    return `
                    <tr class="clickable-row" onclick="window.location.hash='#/session/${s.session_id}'">
                        <td>${label}</td>
                        <td>${shortenProject(s.project)}</td>
                        <td class="mono">${started}</td>
                        <td>${shortenModel(s.model)}</td>
                        <td class="right mono">${formatTokens(s.tokens.total)}</td>
                        ${showCost ? `<td class="right mono">${s.cost ? formatCost(s.cost.total) : ''}</td>` : ''}
                        <td class="right mono">${formatPercent(s.cache_hit_ratio)}</td>
                        <td class="right mono">${s.turns}</td>
                    </tr>`;
                }).join('')}
            </tbody>
        </table>
        <div style="color: var(--text-muted); font-size: 0.8rem; margin-top: 0.5rem;">
            Total: ${formatTokens(snapshot.total_tokens)} tokens
            ${snapshot.total_cost != null ? ' / ' + formatCost(snapshot.total_cost) : ''}
            &mdash; Last update: ${new Date(snapshot.timestamp).toLocaleTimeString()}
        </div>
    `;
}

// Clean up SSE when navigating away from watch page
window.addEventListener('hashchange', () => {
    const hash = window.location.hash || '#/';
    if (!hash.startsWith('#/watch')) {
        cleanupWatch();
    }
    destroyAllCharts();
});

// Pause/resume SSE when tab visibility changes to avoid background CPU/memory use
document.addEventListener('visibilitychange', () => {
    const hash = window.location.hash || '#/';
    if (!hash.startsWith('#/watch')) return;

    if (document.hidden) {
        // Close SSE connection when tab is hidden to stop server-side work
        if (watchEventSource) {
            watchEventSource.close();
            watchEventSource = null;
        }
    } else {
        // Reconnect when tab becomes visible again
        if (!watchEventSource) {
            renderWatchPage();
        }
    }
});
