/**
 * Confetti on Done — a visual outl plugin.
 *
 * Watches the op stream and, whenever a block transitions into DONE, throws a
 * confetti burst. The confetti is written *here, by the plugin author* — the
 * host knows nothing about "confetti". The plugin hands the client a chunk of
 * self-contained HTML/JS (`ctx.ui.render`), which the GUI clients run in a
 * sandboxed iframe overlay. Want fireworks instead? Rewrite CONFETTI_HTML —
 * it's your creativity, not a fixed catalog.
 *
 * Needs the `ui-render` capability (GUI only: desktop + mobile). On the
 * TUI/CLI the render is dropped — the op-hook still fires, there's just no
 * surface to draw on.
 */

import { definePlugin, type LogOp, type PluginContext } from "@outl/plugin-sdk";

/**
 * Self-contained confetti: a full-screen canvas plus a tiny particle
 * simulation. No imports, no network — everything the iframe needs is inline,
 * because the sandbox has neither.
 */
const CONFETTI_HTML = `<!doctype html><html><head><meta charset="utf-8"><style>
  html,body{margin:0;height:100%;overflow:hidden;background:transparent}
  canvas{display:block}
</style></head><body><canvas id="c"></canvas><script>
  const cv=document.getElementById('c'),x=cv.getContext('2d');
  cv.width=innerWidth;cv.height=innerHeight;
  const colors=['#ff595e','#ffca3a','#8ac926','#1982c4','#6a4c93','#ff6ec7'];
  const P=[];
  for(let i=0;i<180;i++){P.push({
    x:innerWidth/2+(Math.random()-.5)*140, y:innerHeight*0.55,
    vx:(Math.random()-.5)*16, vy:Math.random()*-17-5,
    r:4+Math.random()*7, c:colors[i%colors.length], a:1,
    rot:Math.random()*6.28, vr:(Math.random()-.5)*.5
  });}
  let t=0;
  function frame(){
    x.clearRect(0,0,cv.width,cv.height); t++; let alive=false;
    for(const p of P){
      p.vy+=.45; p.x+=p.vx; p.y+=p.vy; p.rot+=p.vr; p.a-=.0075;
      if(p.a>0){
        alive=true; x.save(); x.globalAlpha=Math.max(0,p.a);
        x.translate(p.x,p.y); x.rotate(p.rot); x.fillStyle=p.c;
        x.fillRect(-p.r/2,-p.r/2,p.r,p.r*.6); x.restore();
      }
    }
    if(alive&&t<260) requestAnimationFrame(frame);
  }
  requestAnimationFrame(frame);
</script></body></html>`;

export default definePlugin({
  activate(ctx: PluginContext) {
    ctx.ops.onOp((op: LogOp) => {
      // A TODO→DONE toggle lands as an `Edit` op whose body now carries the
      // DONE prefix, so the host projects `todo: "DONE"` onto it.
      if (op.kind === "Edit" && op.todo === "DONE") {
        ctx.ui.render(CONFETTI_HTML);
      }
    });
  },
});
