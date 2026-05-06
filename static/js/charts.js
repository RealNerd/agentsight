// ── Chart.js Configuration & Wrappers ────────────────

// Global Chart.js defaults for dark theme
Chart.defaults.color = '#8b91a0';
Chart.defaults.borderColor = '#2a2e3a';
Chart.defaults.font.family = "'SF Mono', 'Cascadia Code', monospace";
Chart.defaults.font.size = 11;
Chart.defaults.plugins.legend.labels.usePointStyle = true;
Chart.defaults.plugins.legend.labels.pointStyleWidth = 8;

const COLORS = {
    primary: '#6c8cff',
    green: '#4ade80',
    yellow: '#fbbf24',
    red: '#f87171',
    purple: '#a78bfa',
    orange: '#fb923c',
    cyan: '#22d3ee',
    pink: '#f472b6',
};

const PALETTE = [
    COLORS.primary, COLORS.green, COLORS.yellow, COLORS.purple,
    COLORS.orange, COLORS.cyan, COLORS.pink, COLORS.red,
];

// Track chart instances for cleanup
const chartInstances = {};

function destroyChart(id) {
    if (chartInstances[id]) {
        chartInstances[id].destroy();
        delete chartInstances[id];
    }
}

function destroyAllCharts() {
    Object.keys(chartInstances).forEach(destroyChart);
}

// ── Chart Builders ───────────────────────────────────

function createBarChart(canvasId, labels, datasets, options = {}) {
    destroyChart(canvasId);
    const canvas = document.getElementById(canvasId);
    if (!canvas) return null;

    const chart = new Chart(canvas, {
        type: 'bar',
        data: { labels, datasets },
        options: {
            responsive: true,
            maintainAspectRatio: false,
            plugins: {
                legend: { display: datasets.length > 1 },
                tooltip: {
                    callbacks: {
                        label: (ctx) => {
                            const val = ctx.parsed.y;
                            if (options.formatValue) return ctx.dataset.label + ': ' + options.formatValue(val);
                            return ctx.dataset.label + ': ' + formatTokens(val);
                        }
                    }
                }
            },
            scales: {
                x: { grid: { display: false } },
                y: {
                    beginAtZero: true,
                    ticks: {
                        callback: options.yTickFormat || ((v) => {
                            if (v >= 1_000_000) return (v / 1_000_000).toFixed(1) + 'M';
                            if (v >= 1_000) return (v / 1_000).toFixed(0) + 'K';
                            return v;
                        })
                    }
                }
            },
            ...options.chartOptions,
        }
    });

    chartInstances[canvasId] = chart;
    return chart;
}

function createLineChart(canvasId, labels, datasets, options = {}) {
    destroyChart(canvasId);
    const canvas = document.getElementById(canvasId);
    if (!canvas) return null;

    datasets.forEach(ds => {
        ds.tension = 0.3;
        ds.pointRadius = 3;
        ds.pointHoverRadius = 5;
        if (!ds.borderWidth) ds.borderWidth = 2;
    });

    const chart = new Chart(canvas, {
        type: 'line',
        data: { labels, datasets },
        options: {
            responsive: true,
            maintainAspectRatio: false,
            plugins: {
                legend: { display: datasets.length > 1 },
                tooltip: {
                    callbacks: {
                        label: (ctx) => {
                            const val = ctx.parsed.y;
                            if (options.formatValue) return ctx.dataset.label + ': ' + options.formatValue(val);
                            return ctx.dataset.label + ': ' + formatTokens(val);
                        }
                    }
                }
            },
            scales: {
                x: { grid: { display: false } },
                y: {
                    beginAtZero: true,
                    ticks: {
                        callback: options.yTickFormat || ((v) => {
                            if (v >= 1_000_000) return (v / 1_000_000).toFixed(1) + 'M';
                            if (v >= 1_000) return (v / 1_000).toFixed(0) + 'K';
                            return v;
                        })
                    }
                }
            },
            ...options.chartOptions,
        }
    });

    chartInstances[canvasId] = chart;
    return chart;
}

function createDoughnutChart(canvasId, labels, data, colors = PALETTE) {
    destroyChart(canvasId);
    const canvas = document.getElementById(canvasId);
    if (!canvas) return null;

    const chart = new Chart(canvas, {
        type: 'doughnut',
        data: {
            labels,
            datasets: [{
                data,
                backgroundColor: colors.slice(0, data.length),
                borderWidth: 0,
            }]
        },
        options: {
            responsive: true,
            maintainAspectRatio: false,
            cutout: '65%',
            plugins: {
                legend: {
                    position: 'right',
                    labels: { padding: 12 }
                },
                tooltip: {
                    callbacks: {
                        label: (ctx) => {
                            const total = ctx.dataset.data.reduce((a, b) => a + b, 0);
                            const pct = total > 0 ? ((ctx.parsed / total) * 100).toFixed(1) : 0;
                            return ctx.label + ': ' + formatTokens(ctx.parsed) + ' (' + pct + '%)';
                        }
                    }
                }
            }
        }
    });

    chartInstances[canvasId] = chart;
    return chart;
}
