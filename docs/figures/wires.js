// Draws the arrows of a figure from element boxes, so the layout can change without redrawing.
// wire(from, fromSide, to, toSide, opts): sides are left|right|top|bottom; opts.at / opts.at2 move
// the anchor along the side (0..1); opts.y (opts.x) pins a horizontal (vertical) wire to that line;
// opts.drop routes a loop below both boxes by that many px. A `.model` circle is met at its edge.
(function () {
  const fig = document.querySelector('.fig');
  const svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
  svg.setAttribute('class', 'wires');
  svg.innerHTML = '<defs><marker id="ah" viewBox="0 0 10 10" refX="9" refY="5" markerWidth="7" markerHeight="7" orient="auto-start-reverse"><path d="M0 0 10 5 0 10z" fill="#555"/></marker></defs>';
  fig.appendChild(svg);
  const F = fig.getBoundingClientRect();
  function pt(id, side, t, o) {
    const el = document.getElementById(id), r = el.getBoundingClientRect();
    const x = r.left - F.left, y = r.top - F.top;
    t = t ?? 0.5;
    let p = { left: [x, y + r.height * t], right: [x + r.width, y + r.height * t], top: [x + r.width * t, y], bottom: [x + r.width * t, y + r.height] }[side];
    if (o.y !== undefined && (side === 'left' || side === 'right')) p = [p[0], o.y];
    if (o.x !== undefined && (side === 'top' || side === 'bottom')) p = [o.x, p[1]];
    if (el.classList.contains('model')) {
      const R = r.width / 2, cx = x + R, cy = y + R;
      if (side === 'left' || side === 'right') { const dx = Math.sqrt(Math.max(R * R - (p[1] - cy) ** 2, 0)); p[0] = side === 'left' ? cx - dx : cx + dx; }
      else { const dy = Math.sqrt(Math.max(R * R - (p[0] - cx) ** 2, 0)); p[1] = side === 'top' ? cy - dy : cy + dy; }
    }
    return p;
  }
  window.wire = function (a, sa, b, sb, o = {}) {
    const [x1, y1] = pt(a, sa, o.at, o), [x2, y2] = pt(b, sb, o.at2, o);
    let d, mid;
    if (o.drop) {
      const y = Math.max(y1, y2) + o.drop;
      d = `M${x1} ${y1} V${y} H${x2} V${y2}`; mid = [(x1 + x2) / 2, y];
    } else if ((sa === 'right' || sa === 'left') && (sb === 'left' || sb === 'right')) {
      const mx = o.mx ?? (x1 + x2) / 2;
      d = Math.abs(y1 - y2) < 1 ? `M${x1} ${y1} H${x2}` : `M${x1} ${y1} H${mx} V${y2} H${x2}`; mid = [mx, (y1 + y2) / 2];
      if (Math.abs(y1 - y2) < 1) mid = [(x1 + x2) / 2, y1];
    } else if ((sa === 'top' || sa === 'bottom') && (sb === 'top' || sb === 'bottom')) {
      const my = o.my ?? (y1 + y2) / 2;
      d = Math.abs(x1 - x2) < 1 ? `M${x1} ${y1} V${y2}` : `M${x1} ${y1} V${my} H${x2} V${y2}`; mid = [(x1 + x2) / 2, my];
      if (Math.abs(x1 - x2) < 1) mid = [x1, (y1 + y2) / 2];
    } else if (sa === 'right' || sa === 'left') {
      d = `M${x1} ${y1} H${x2} V${y2}`; mid = [(x1 + x2) / 2, y1];
    } else {
      d = `M${x1} ${y1} V${y2} H${x2}`; mid = [x1, (y1 + y2) / 2];
    }
    const p = document.createElementNS('http://www.w3.org/2000/svg', 'path');
    p.setAttribute('d', d); p.setAttribute('fill', 'none'); p.setAttribute('stroke', o.color || '#555');
    p.setAttribute('stroke-width', o.w || 2); p.setAttribute('marker-end', 'url(#ah)');
    if (o.dashed) p.setAttribute('stroke-dasharray', '6 5');
    svg.appendChild(p);
    if (o.label) {
      const l = document.createElement('div');
      l.className = 'wlabel' + (o.bare ? ' bare' : '');
      l.innerHTML = o.label;
      const [lx, ly] = o.lpos ? [mid[0] + o.lpos[0], mid[1] + o.lpos[1]] : mid;
      l.style.left = lx + 'px'; l.style.top = ly + 'px';
      fig.appendChild(l);
    }
  };
})();
