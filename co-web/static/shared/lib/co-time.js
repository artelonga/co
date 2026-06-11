// CO-387: time-rendering primitive — pure per-lens conversion math.
// Mirror of co-web/src/time/conversion.rs — keep both in sync.
//
// A lens reads ONE canonical field from the entry and maps it to a position.
// Canonical units are per-lens:
//   i64_ms    — Unix milliseconds (Gregorian, 4-day-week, Pomodoro, fictional-ms)
//   f64_years — fractional years-before-present, log scale (cosmic)
//   i64_units — raw linear unit, e.g. a fictional year number (shandara)

export const MS_PER_DAY = 86400000;
export const MS_PER_HOUR = 3600000;

export function defaultCalendarConfig() {
    return {
        default_lens: 'gregorian',
        lenses: [{
            id: 'gregorian',
            name: 'Gregorian (canonical)',
            epoch_ms: 0,
            scale: 'linear',
            canonical_field: 'event_at_ms',
            canonical_type: 'i64_ms',
            week_length_days: 7,
            weekday_names: ['Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat', 'Sun'],
            month_length: 'gregorian',
        }],
    };
}

// Resolution: custom_event_field → canonical_field → event_at_ms.
export function resolvedCanonicalField(lens) {
    return lens.custom_event_field || lens.canonical_field || 'event_at_ms';
}

// Explicit canonical_type wins; custom_event_field implies raw units; else ms.
export function resolvedCanonicalType(lens) {
    if (lens.canonical_type) return lens.canonical_type;
    if (lens.custom_event_field) return 'i64_units';
    return 'i64_ms';
}

// The ISO frontmatter sources for each ms fast-path field. The server keeps
// entries.event_at_ms etc. in sync (universe-pool v19), but the entries API
// payload carries frontmatter — so the client parses the ISO twin. Extra
// aliases cover task-style frontmatter (due_date / start_date / date).
const MS_FIELD_SOURCES = {
    event_at_ms: ['event_at', 'date'],
    due_at_ms: ['due_at', 'due_date'],
    scheduled_at_ms: ['scheduled_at', 'start_date'],
};

// Read the lens' canonical value from an entry. Returns a number or null.
export function readCanonicalValue(entry, lens) {
    const field = resolvedCanonicalField(lens);
    const fm = entry.frontmatter || entry.payload || entry || {};
    const sources = MS_FIELD_SOURCES[field];
    if (sources) {
        // ms column field → parse the ISO twin from frontmatter.
        for (const key of sources) {
            const raw = fm[key] !== undefined ? fm[key] : entry[key];
            if (typeof raw === 'number') return raw;
            if (typeof raw === 'string' && raw) {
                const ms = Date.parse(/T/.test(raw) ? raw : raw + 'T00:00:00Z');
                if (!Number.isNaN(ms)) return ms;
            }
        }
        return null;
    }
    const v = fm[field];
    return (typeof v === 'number' && Number.isFinite(v)) ? v : null;
}

// Map a canonical value to a lens position. null = type/scale mismatch —
// the renderer shows the entry as an orphan.
export function entryToLensPosition(raw, lens) {
    if (raw === null || raw === undefined || !Number.isFinite(raw)) return null;
    const scale = lens.scale || 'linear';
    const ctype = resolvedCanonicalType(lens);

    if (scale === 'linear' && ctype === 'i64_ms') {
        const offset = raw - (lens.epoch_ms || 0);
        if (lens.cell_duration_ms) {
            // Pomodoro-style: cycle = work cell + break.
            const cellMs = Math.max(lens.cell_duration_ms, 1);
            const breakMs = Math.max(lens.break_duration_ms || 0, 0);
            const cycle = cellMs + breakMs;
            const cell = Math.floor(offset / cycle);
            const within = offset - cell * cycle;
            return { kind: 'cell', cell, in_break: within >= cellMs };
        }
        const day = Math.floor(offset / MS_PER_DAY);
        const weekLen = Math.max(lens.week_length_days || 7, 1);
        return {
            kind: 'linear',
            week: Math.floor(day / weekLen),
            day_of_week: ((day % weekLen) + weekLen) % weekLen,
            hour: ((Math.floor(offset / MS_PER_HOUR) % 24) + 24) % 24,
        };
    }
    if (scale === 'linear' && ctype === 'i64_units') {
        return { kind: 'axis', position: raw - (lens.epoch_ms || 0) };
    }
    if (scale === 'log' && ctype === 'f64_years') {
        // log10 placement; closer to present = smaller.
        return { kind: 'log', log_position: Math.log10(Math.max(raw, 1)) };
    }
    return null;
}

// Inverse mapping — "click a cell to create an event there".
export function lensPositionToCanonical(pos, lens) {
    if (!pos) return null;
    const ctype = resolvedCanonicalType(lens);
    if (pos.kind === 'linear' && ctype === 'i64_ms') {
        const weekLen = Math.max(lens.week_length_days || 7, 1);
        const day = pos.week * weekLen + pos.day_of_week;
        return (lens.epoch_ms || 0) + day * MS_PER_DAY + (pos.hour || 0) * MS_PER_HOUR;
    }
    if (pos.kind === 'cell' && ctype === 'i64_ms' && lens.cell_duration_ms) {
        const cycle = lens.cell_duration_ms + Math.max(lens.break_duration_ms || 0, 0);
        return (lens.epoch_ms || 0) + pos.cell * cycle;
    }
    if (pos.kind === 'axis' && ctype === 'i64_units') {
        return Math.round(pos.position) + (lens.epoch_ms || 0);
    }
    if (pos.kind === 'log' && ctype === 'f64_years') {
        return Math.pow(10, pos.log_position);
    }
    return null;
}

// Format a position for display, honoring lens.display_format placeholders
// {week} {day} {cell} (e.g. "S{week}.D{day}", "Pom{cell}").
export function formatLensPosition(pos, lens) {
    if (!pos) return '';
    const fmt = lens.display_format;
    if (fmt) {
        return fmt
            .replace('{week}', pos.week !== undefined ? pos.week : '')
            .replace('{day}', pos.day_of_week !== undefined ? pos.day_of_week : '')
            .replace('{cell}', pos.cell !== undefined ? pos.cell : '');
    }
    if (pos.kind === 'linear') {
        const names = lens.weekday_names || [];
        const dayName = names[pos.day_of_week] || ('D' + pos.day_of_week);
        return 'W' + pos.week + ' · ' + dayName;
    }
    if (pos.kind === 'cell') return 'Cell ' + pos.cell + (pos.in_break ? ' (break)' : '');
    if (pos.kind === 'axis') return String(pos.position);
    if (pos.kind === 'log') {
        const years = Math.pow(10, pos.log_position);
        if (years >= 1e9) return (years / 1e9).toFixed(2) + ' Gyr BP';
        if (years >= 1e6) return (years / 1e6).toFixed(2) + ' Myr BP';
        return Math.round(years).toLocaleString() + ' yr BP';
    }
    return '';
}
