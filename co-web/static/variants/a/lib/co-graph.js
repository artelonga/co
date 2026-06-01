/* co-graph.js — CO-335: canonical graph rendering primitive.
 *
 * Single source of truth for graph rendering across CO sister sites.
 * Canvas-based force-directed (or manual) layout, CSS-variable theming,
 * built-in pan/zoom/hover/click. No external dependencies.
 *
 * API:
 *   const handle = co_graph.render(container, opts);
 *
 *   opts = {
 *     data: {
 *       nodes: [{ id, label, sublabel?, color?, size?, selected?, x?, y?, fixed? }],
 *       edges: [{ source, target, kind?, label?, color?, directed? }]
 *     },
 *     layout: 'force' | 'manual',  // default: 'force'
 *     grid: false | { size: 60, color: '...' },
 *     onNodeClick:      (node, screenX, screenY) => {},
 *     onNodeHover:      (node | null) => {},
 *     onConnectionEnd:  (fromNode, toNode) => {},  // drag node → node
 *     onNodeMoveEnd:    (node) => {},               // manual layout drag
 *   }
 *
 *   handle = {
 *     update({ nodes, edges }),
 *     fit(animate),
 *     getNodeAt(sx, sy),
 *     worldToScreen(x, y) → [sx, sy],
 *     screenToWorld(sx, sy) → [x, y],
 *     destroy()
 *   }
 *
 * CSS variables (set on the container or any ancestor):
 *   --co-graph-node-default    node fill when no color set  (#8090a0)
 *   --co-graph-edge-color      default edge stroke          (rgba(180,190,200,0.25))
 *   --co-graph-label-color     node label text              (#c8ced4)
 *   --co-graph-sublabel-color  gloss / secondary label      (#9a937f)
 *   --co-graph-hover-ring      ring around hovered node     (#d4af37)
 *   --co-graph-selected-ring   ring around selected node    (#d4af37)
 *   --co-graph-grid-color      grid line color              (rgba(200,200,200,0.08))
 */
(function (root) {
  'use strict';

  function render(container, opts) {
    opts = opts || {};
    var layout     = opts.layout  || 'force';
    var gridOpts   = opts.grid    || false;
    var onNodeClick     = opts.onNodeClick     || null;
    var onNodeHover     = opts.onNodeHover     || null;
    var onConnectionEnd = opts.onConnectionEnd || null;
    var onNodeMoveEnd   = opts.onNodeMoveEnd   || null;

    // ── Canvas ────────────────────────────────────────────────────────────────
    var canvas = document.createElement('canvas');
    canvas.style.cssText = 'position:absolute;top:0;left:0;width:100%;height:100%;display:block;cursor:default;touch-action:none;';
    if (getComputedStyle(container).position === 'static') {
      container.style.position = 'relative';
    }
    container.appendChild(canvas);
    var ctx = canvas.getContext('2d');
    var W = 0, H = 0;
    var dpr = Math.max(1, Math.min(2, window.devicePixelRatio || 1));

    function resize() {
      W = container.clientWidth  || 1;
      H = container.clientHeight || 1;
      canvas.width  = W * dpr;
      canvas.height = H * dpr;
      canvas.style.width  = W + 'px';
      canvas.style.height = H + 'px';
      ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    }

    // ── Camera ────────────────────────────────────────────────────────────────
    var cam = { x: 0, y: 0, s: 1 };
    var camTarget = null;

    function w2s(x, y) {
      return [(x - cam.x) * cam.s + W / 2, (y - cam.y) * cam.s + H / 2];
    }
    function s2w(sx, sy) {
      return [(sx - W / 2) / cam.s + cam.x, (sy - H / 2) / cam.s + cam.y];
    }

    // ── Node / Edge state ─────────────────────────────────────────────────────
    var nodes = [];   // internal nodes with physics state
    var edges = [];   // internal edges
    var byId  = {};   // id → internal node

    function initData(d) {
      var prevById = byId;
      var newNodes = (d.nodes || []).map(function (n) {
        var prev = prevById[n.id];
        return {
          id:       n.id,
          label:    n.label    != null ? n.label    : n.id,
          sublabel: n.sublabel != null ? n.sublabel : null,
          color:    n.color    || null,
          size:     n.size     || 8,
          fixed:    !!n.fixed,
          selected: !!n.selected,
          // Preserve physics position across updates; seed from data or random
          x:  prev ? prev.x : (n.x != null ? n.x : (Math.random() - 0.5) * 300),
          y:  prev ? prev.y : (n.y != null ? n.y : (Math.random() - 0.5) * 300),
          vx: prev ? prev.vx : 0,
          vy: prev ? prev.vy : 0,
          fx: 0, fy: 0,
          _data: n, // original passed-in object; x/y updated on move
        };
      });
      var newEdges = (d.edges || []).map(function (e) {
        return {
          source:   e.source,
          target:   e.target,
          kind:     e.kind     || 'default',
          label:    e.label    || null,
          color:    e.color    || null,
          directed: !!e.directed,
        };
      });

      nodes = newNodes;
      edges = newEdges;
      byId  = {};
      nodes.forEach(function (n) { byId[n.id] = n; });

      if (layout === 'force') alpha = 1;
    }

    initData(opts.data || { nodes: [], edges: [] });

    // ── Physics (force-directed, only when layout === 'force') ────────────────
    var alpha = 1;

    function stepPhysics() {
      if (alpha < 0.02) return;
      alpha *= 0.97;
      var i, j, n = nodes.length;
      for (i = 0; i < n; i++) { nodes[i].fx = 0; nodes[i].fy = 0; }

      // Pairwise repulsion
      for (i = 0; i < n; i++) {
        for (j = i + 1; j < n; j++) {
          var a = nodes[i], b = nodes[j];
          var dx = a.x - b.x, dy = a.y - b.y;
          var d2 = dx * dx + dy * dy + 0.01;
          var d  = Math.sqrt(d2);
          var f  = 3200 / d2;
          var ux = dx / d, uy = dy / d;
          a.fx += ux * f; a.fy += uy * f;
          b.fx -= ux * f; b.fy -= uy * f;
        }
      }

      // Spring forces along edges
      edges.forEach(function (e) {
        var a = byId[e.source], b = byId[e.target];
        if (!a || !b) return;
        var dx = b.x - a.x, dy = b.y - a.y;
        var d  = Math.sqrt(dx * dx + dy * dy) || 1;
        var rest = 80;
        var f  = (d - rest) * 0.03;
        var ux = dx / d, uy = dy / d;
        a.fx += ux * f; a.fy += uy * f;
        b.fx -= ux * f; b.fy -= uy * f;
      });

      // Gravity toward origin + Euler integration
      for (i = 0; i < n; i++) {
        var a = nodes[i];
        if (a.fixed) { a.x = 0; a.y = 0; a.vx = 0; a.vy = 0; continue; }
        a.fx += -a.x * 0.012;
        a.fy += -a.y * 0.012;
        a.vx = (a.vx + a.fx) * 0.82;
        a.vy = (a.vy + a.fy) * 0.82;
        a.x  += a.vx * 0.25;
        a.y  += a.vy * 0.25;
      }
    }

    // ── CSS variable helper ────────────────────────────────────────────────────
    var _cssCache = {};
    function cssVar(name, fallback) {
      if (_cssCache[name]) return _cssCache[name];
      var v = getComputedStyle(container).getPropertyValue(name).trim();
      if (v) { _cssCache[name] = v; return v; }
      return fallback;
    }
    // Invalidate cache on re-render (theme may change)
    function clearCssCache() { _cssCache = {}; }

    // ── Draw ──────────────────────────────────────────────────────────────────
    var LABEL_Z   = 0.45;  // zoom ≥ this to show labels
    var GLOSS_Z   = 1.2;   // zoom ≥ this to show sublabels

    function draw() {
      clearCssCache();

      // Smooth camera animation
      if (camTarget) {
        cam.x += (camTarget.x - cam.x) * 0.12;
        cam.y += (camTarget.y - cam.y) * 0.12;
        cam.s += (camTarget.s - cam.s) * 0.12;
        if (Math.abs(camTarget.x - cam.x) < 0.4 &&
            Math.abs(camTarget.y - cam.y) < 0.4 &&
            Math.abs(camTarget.s - cam.s) < 0.01) {
          camTarget = null;
        }
      }

      ctx.clearRect(0, 0, W, H);

      // Grid background
      if (gridOpts) {
        var gs = (gridOpts.size || 60) * cam.s;
        if (gs > 10) {
          ctx.strokeStyle = gridOpts.color || cssVar('--co-graph-grid-color', 'rgba(200,200,200,0.08)');
          ctx.lineWidth = 1;
          var o = w2s(0, 0);
          for (var gx = ((o[0] % gs) + gs) % gs; gx < W; gx += gs) {
            ctx.beginPath(); ctx.moveTo(gx, 0); ctx.lineTo(gx, H); ctx.stroke();
          }
          for (var gy = ((o[1] % gs) + gs) % gs; gy < H; gy += gs) {
            ctx.beginPath(); ctx.moveTo(0, gy); ctx.lineTo(W, gy); ctx.stroke();
          }
        }
      }

      var edgeColor    = cssVar('--co-graph-edge-color',    'rgba(180,190,200,0.25)');
      var nodeDefault  = cssVar('--co-graph-node-default',  '#8090a0');
      var labelColor   = cssVar('--co-graph-label-color',   '#c8ced4');
      var sublabelColor = cssVar('--co-graph-sublabel-color','#9a937f');
      var hoverRing    = cssVar('--co-graph-hover-ring',    '#d4af37');
      var selectedRing = cssVar('--co-graph-selected-ring', '#d4af37');

      // Edges
      edges.forEach(function (e) {
        var a = byId[e.source], b = byId[e.target];
        if (!a || !b) return;
        var p = w2s(a.x, a.y), q = w2s(b.x, b.y);
        ctx.strokeStyle = e.color || edgeColor;
        ctx.lineWidth = 1.4;
        ctx.beginPath(); ctx.moveTo(p[0], p[1]); ctx.lineTo(q[0], q[1]); ctx.stroke();

        // Arrowhead for directed edges
        if (e.directed) {
          var dx = q[0] - p[0], dy = q[1] - p[1];
          var len = Math.hypot(dx, dy) || 1;
          var ux = dx / len, uy = dy / len;
          var nr = b.size * cam.s + 2;
          var tx = q[0] - ux * nr, ty = q[1] - uy * nr;
          var s = 8, w = 0.55;
          ctx.fillStyle = e.color || edgeColor;
          ctx.beginPath();
          ctx.moveTo(tx, ty);
          ctx.lineTo(tx - ux * s - uy * s * w, ty - uy * s + ux * s * w);
          ctx.lineTo(tx - ux * s + uy * s * w, ty - uy * s - ux * s * w);
          ctx.closePath();
          ctx.fill();
        }

        // Edge label
        if (e.label && cam.s > 0.5) {
          ctx.fillStyle = e.color || labelColor;
          ctx.font = '10px system-ui,sans-serif';
          ctx.textAlign = 'center';
          ctx.textBaseline = 'middle';
          ctx.fillText(e.label.slice(0, 40), (p[0] + q[0]) / 2, (p[1] + q[1]) / 2 - 5);
        }
      });

      // Nodes
      var showLabels = cam.s >= LABEL_Z;
      nodes.forEach(function (a) {
        var p = w2s(a.x, a.y);
        var r = a.size * cam.s;
        var isHover    = (a === hoveredNode);
        var isSelected = a.selected;

        // Selection ring (behind node fill)
        if (isSelected) {
          ctx.beginPath(); ctx.arc(p[0], p[1], r + 5, 0, 6.2832);
          ctx.strokeStyle = selectedRing; ctx.lineWidth = 2; ctx.stroke();
        }

        // Node circle
        ctx.beginPath(); ctx.arc(p[0], p[1], r, 0, 6.2832);
        ctx.fillStyle = isHover ? '#ffffff' : (a.color || nodeDefault);
        ctx.fill();

        // Hover ring
        if (isHover && !isSelected) {
          ctx.beginPath(); ctx.arc(p[0], p[1], r + 3, 0, 6.2832);
          ctx.strokeStyle = hoverRing; ctx.lineWidth = 1.5; ctx.stroke();
        }

        // Main label
        if ((showLabels || isHover || isSelected) && a.label) {
          var px = Math.max(10, Math.min(22, 13 * cam.s));
          ctx.font = '600 ' + px + 'px system-ui,sans-serif';
          ctx.textAlign = 'center';
          ctx.textBaseline = 'top';
          ctx.fillStyle = labelColor;
          ctx.fillText(a.label, p[0], p[1] + r + 3);

          // Sublabel (gloss)
          if (a.sublabel && cam.s >= GLOSS_Z) {
            var spx = Math.max(8, Math.min(13, 10 * cam.s));
            ctx.font = spx + 'px system-ui,sans-serif';
            ctx.fillStyle = sublabelColor;
            ctx.fillText(a.sublabel, p[0], p[1] + r + 3 + px + 2);
          }
        }
      });

      // Connection-in-progress preview line
      if (conn.from && conn.ptr) {
        var f = w2s(conn.from.x, conn.from.y);
        ctx.strokeStyle = 'rgba(192,57,43,0.8)';
        ctx.setLineDash([5, 4]);
        ctx.lineWidth = 2;
        ctx.beginPath(); ctx.moveTo(f[0], f[1]); ctx.lineTo(conn.ptr[0], conn.ptr[1]); ctx.stroke();
        ctx.setLineDash([]);
      }
    }

    // ── RAF loop ──────────────────────────────────────────────────────────────
    var rafId = null;
    function loop() {
      if (layout === 'force') stepPhysics();
      draw();
      rafId = requestAnimationFrame(loop);
    }

    // ── Fit ───────────────────────────────────────────────────────────────────
    function fit(animate) {
      if (!nodes.length) return;
      var mnX = 1e9, mnY = 1e9, mxX = -1e9, mxY = -1e9;
      nodes.forEach(function (a) {
        mnX = Math.min(mnX, a.x); mnY = Math.min(mnY, a.y);
        mxX = Math.max(mxX, a.x); mxY = Math.max(mxY, a.y);
      });
      var bw = Math.max(160, mxX - mnX + 120);
      var bh = Math.max(160, mxY - mnY + 120);
      var v = {
        x: (mnX + mxX) / 2,
        y: (mnY + mxY) / 2,
        s: Math.max(0.1, Math.min(2.5, Math.min(W / bw, H / bh))),
      };
      if (animate) { camTarget = v; } else { cam = { x: v.x, y: v.y, s: v.s }; }
    }

    // ── Hit testing ────────────────────────────────────────────────────────────
    function getNodeAt(sx, sy) {
      var best = null, bd = 1e9;
      nodes.forEach(function (a) {
        var p = w2s(a.x, a.y);
        var d = Math.hypot(p[0] - sx, p[1] - sy);
        var rr = Math.max(10, a.size * cam.s + 6);
        if (d < rr && d < bd) { bd = d; best = a; }
      });
      return best;
    }

    // ── Pointer interaction ────────────────────────────────────────────────────
    var hoveredNode = null;
    var conn        = { from: null, ptr: null, moved: false };
    var panning     = false;
    var downPos     = null;
    var lastPos     = null;
    var dragging    = null; // { node, moved } — manual layout node drag

    function canvasPos(e) {
      var rc = canvas.getBoundingClientRect();
      return [
        (e.clientX - rc.left) * (W / rc.width),
        (e.clientY - rc.top)  * (H / rc.height),
      ];
    }

    canvas.addEventListener('pointerdown', function (e) {
      if (e.button !== 0 && e.button !== undefined) return;
      canvas.setPointerCapture && canvas.setPointerCapture(e.pointerId);
      var p = canvasPos(e);
      downPos = p; lastPos = p; conn.moved = false;
      var h = getNodeAt(p[0], p[1]);
      if (h) {
        conn.from = h;
        panning = false;
        if (layout === 'manual') {
          dragging = { node: h, moved: false };
        }
      } else {
        conn.from = null;
        panning   = true;
        dragging  = null;
      }
      conn.ptr = null;
    });

    canvas.addEventListener('pointermove', function (e) {
      var p = canvasPos(e);
      if (downPos && Math.hypot(p[0] - downPos[0], p[1] - downPos[1]) > 4) {
        conn.moved = true;
      }

      if (conn.from && conn.moved) {
        if (layout === 'manual' && dragging) {
          var w = s2w(p[0], p[1]);
          dragging.node.x = w[0];
          dragging.node.y = w[1];
          dragging.moved = true;
          canvas.style.cursor = 'grabbing';
        } else {
          conn.ptr = p;
          canvas.style.cursor = 'crosshair';
        }
      } else if (panning && conn.moved) {
        cam.x -= (p[0] - lastPos[0]) / cam.s;
        cam.y -= (p[1] - lastPos[1]) / cam.s;
        cam.x = Math.max(-3000, Math.min(3000, cam.x));
        cam.y = Math.max(-3000, Math.min(3000, cam.y));
        camTarget = null;
        canvas.style.cursor = 'grabbing';
      } else if (!conn.from && !panning) {
        var h = getNodeAt(p[0], p[1]);
        if (h !== hoveredNode) {
          hoveredNode = h;
          if (onNodeHover) onNodeHover(h ? h._data : null);
        }
        canvas.style.cursor = h ? 'pointer' : '';
      }

      lastPos = p;
    });

    canvas.addEventListener('pointerup', function (e) {
      canvas.style.cursor = '';
      var p = canvasPos(e);

      if (conn.from) {
        if (conn.moved) {
          if (layout === 'manual' && dragging && dragging.moved) {
            // Update original data object with final position
            dragging.node._data.x = dragging.node.x;
            dragging.node._data.y = dragging.node.y;
            if (onNodeMoveEnd) onNodeMoveEnd(dragging.node._data);
          } else if (onConnectionEnd) {
            var t = getNodeAt(p[0], p[1]);
            if (t && t !== conn.from) {
              onConnectionEnd(conn.from._data, t._data);
            }
          }
        } else {
          if (onNodeClick) onNodeClick(conn.from._data, p[0], p[1]);
        }
      }

      conn.from  = null;
      conn.ptr   = null;
      conn.moved = false;
      dragging   = null;
      downPos    = null;
      panning    = false;
    });

    canvas.addEventListener('pointercancel', function () {
      conn.from = null; conn.ptr = null; conn.moved = false;
      dragging = null; downPos = null; panning = false;
    });

    canvas.addEventListener('wheel', function (e) {
      e.preventDefault();
      var p = canvasPos(e);
      var wx = (p[0] - W / 2) / cam.s + cam.x;
      var wy = (p[1] - H / 2) / cam.s + cam.y;
      cam.s = Math.max(0.1, Math.min(8, cam.s * Math.exp(-e.deltaY * 0.0015)));
      cam.x = wx - (p[0] - W / 2) / cam.s;
      cam.y = wy - (p[1] - H / 2) / cam.s;
      camTarget = null;
    }, { passive: false });

    // ── Pinch zoom (touch) ────────────────────────────────────────────────────
    var pinch = null;
    canvas.addEventListener('touchstart', function (e) {
      if (e.touches.length === 2) {
        e.preventDefault();
        var t0 = e.touches[0], t1 = e.touches[1];
        var dist = Math.hypot(t1.clientX - t0.clientX, t1.clientY - t0.clientY);
        var rect = canvas.getBoundingClientRect();
        var cxs = (t0.clientX + t1.clientX) / 2 - rect.left;
        var cys = (t0.clientY + t1.clientY) / 2 - rect.top;
        var wxy = s2w(cxs, cys);
        pinch = { dist0: dist, s0: cam.s, wx: wxy[0], wy: wxy[1] };
      }
    }, { passive: false });
    canvas.addEventListener('touchmove', function (e) {
      if (pinch && e.touches.length === 2) {
        e.preventDefault();
        var t0 = e.touches[0], t1 = e.touches[1];
        var dist = Math.hypot(t1.clientX - t0.clientX, t1.clientY - t0.clientY);
        var rect = canvas.getBoundingClientRect();
        var cxs = (t0.clientX + t1.clientX) / 2 - rect.left;
        var cys = (t0.clientY + t1.clientY) / 2 - rect.top;
        var newS = Math.max(0.1, Math.min(8, pinch.s0 * (dist / pinch.dist0)));
        cam.s = newS;
        cam.x = pinch.wx - (cxs - W / 2) / newS;
        cam.y = pinch.wy - (cys - H / 2) / newS;
        camTarget = null;
      }
    }, { passive: false });
    canvas.addEventListener('touchend', function () { pinch = null; }, { passive: true });

    // ── Resize observer ────────────────────────────────────────────────────────
    var ro = null;
    if (typeof ResizeObserver !== 'undefined') {
      ro = new ResizeObserver(function () { resize(); });
      ro.observe(container);
    } else {
      window.addEventListener('resize', resize, { passive: true });
    }

    // ── Bootstrap ─────────────────────────────────────────────────────────────
    resize();
    if (layout === 'force') {
      // Let physics run briefly before fitting, so nodes spread out
      setTimeout(function () { fit(false); alpha = 1; }, 80);
    } else {
      fit(false);
    }
    loop();

    // ── Public handle ──────────────────────────────────────────────────────────
    return {
      update: function (newData) {
        initData(newData);
        if (layout !== 'force') fit(false);
      },
      fit: fit,
      getNodeAt: getNodeAt,
      worldToScreen: function (x, y) { return w2s(x, y); },
      screenToWorld: function (sx, sy) { return s2w(sx, sy); },
      getCamera: function () { return { x: cam.x, y: cam.y, s: cam.s }; },
      setCamera: function (c, animate) {
        if (animate) { camTarget = { x: c.x, y: c.y, s: c.s }; }
        else { cam.x = c.x; cam.y = c.y; cam.s = c.s; camTarget = null; }
      },
      destroy: function () {
        cancelAnimationFrame(rafId);
        rafId = null;
        if (ro) { ro.disconnect(); }
        else { window.removeEventListener('resize', resize); }
        canvas.remove();
      },
    };
  }

  root.co_graph = { render: render };

})(typeof window !== 'undefined' ? window : this);
