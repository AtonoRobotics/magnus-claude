// AT-17/19: tokens, composited chrome, migration and real preference API.
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import vm from 'node:vm';
const source = readFileSync(new URL('../ui/theme.js', import.meta.url), 'utf8');
const context = {}; vm.runInNewContext(source, context); const T = context.Theme;
assert.ok(T.PRESETS, 'four presets must be implemented');
for (const theme of ['light', 'dark', 'ember', 'corporate']) for (const material of ['solid', 'glass']) {
  const look = T.effective({ ...T.V2_DEFAULTS, theme, followComputer: false, material }, {}), t = T.tokens(look);
  assert.equal(look.theme, theme);
  for (const fg of ['fg', 'muted', 'accent-text']) for (const bg of ['canvas', 'raised', 'row-sel']) assert.ok(T.contrast(t[fg], t[bg]) >= 4.5, `${theme} ${fg}/${bg}`);
  assert.ok(T.contrast(t['accent-fg'], t.accent) >= 4.5);
  for (const fg of ['chrome-text', 'chrome-muted', 'chrome-focus']) for (const bg of ['chrome-surface', 'chrome-selected']) assert.ok(T.contrast(t[fg], t[bg]) >= 4.5, `${theme} ${fg}/${bg}`);
  if (material === 'glass') {
    const rgba = t.side.match(/[\d.]+/g).map(Number); assert.ok(rgba[3] >= .9);
    for (const back of [0, 255]) {
      const bg = '#' + rgba.slice(0, 3).map(v => Math.round(v * rgba[3] + back * (1-rgba[3])).toString(16).padStart(2, '0')).join('');
      for (const fg of ['chrome-text', 'chrome-muted', 'chrome-focus']) assert.ok(T.contrast(t[fg], bg) >= 4.5, `${theme} glass ${fg}/${back}`);
    }
  }
}
for (const mode of ['light', 'dark', 'system']) for (const material of ['solid', 'glass']) for (const accent of T.ACCENTS.map(a => a.id)) for (const motion of ['full', 'reduced', 'off']) {
  const old = { mode, material, accent, motion }, p = T.preferences(null, old);
  assert.equal(p.theme, 'custom'); for (const [k,v] of Object.entries(old)) assert.equal(p[k], v);
  assert.equal(p.followComputer, mode === 'system');
}
const clay = T.ACCENTS.find(a => a.id === 'clay'), originalClay = {...clay.light};
clay.light.accent = '#FFFFFF'; clay.light['accent-fg'] = '#FFFFFF'; clay.light['accent-text'] = '#FFFFFF';
const adjusted = T.tokens(T.effective({...T.V2_DEFAULTS,theme:'custom',followComputer:false,mode:'light',accent:'clay'},{}));
assert.equal(adjusted['contrast-adjusted'],'true'); assert.ok(T.contrast(adjusted['accent-fg'],adjusted.accent)>=4.5);
Object.assign(clay.light, originalClay);
assert.equal(T.preferences({version:2, theme:'invalid', conversationWidthPx:9999}).conversationWidthPx, 560);
assert.equal(T.effective({...T.V2_DEFAULTS,theme:'ember',followComputer:false},{dark:false}).theme,'ember');
for (const os of [{reduceTransparency:true},{blur:false},{forcedColors:true}]) assert.equal(T.effective({...T.V2_DEFAULTS, material:'glass'},os).material,'solid');
const store = new Map([['magnus.appearance', JSON.stringify({mode:'dark',accent:'clay',material:'glass',motion:'off'})]]);
let fail = false; const events=[];
const browser = { localStorage: { getItem:k=>store.get(k)??null, setItem:(k,v)=>{if(fail)throw Error('disk');store.set(k,v);} },
  document: {documentElement:{style:{setProperty(){}},dataset:{}},querySelectorAll:()=>[]},
  addEventListener(){},dispatchEvent:e=>events.push(e),CustomEvent:class{constructor(name,opts){this.type=name;this.detail=opts.detail;}} };
browser.window=browser; vm.runInNewContext(source,browser);const U=browser.Theme;
U.scope('appliance-a','alice');assert.equal(U.prefs().accent,'clay'); U.set({theme:'corporate'});assert.equal(U.prefs().followComputer,false);
U.scope('appliance-b','alice');assert.equal(U.prefs().theme,'custom');U.set({theme:'ember'});
U.scope('appliance-a','bob');assert.equal(U.prefs().theme,'custom');U.scope('appliance-a','alice');assert.equal(U.prefs().theme,'corporate');
fail=true;U.set({theme:'dark'});assert.equal(U.saved,false);assert.equal(U.look.theme,'dark');assert.equal(JSON.parse(store.get('magnus.ui.v2.appliance-a.alice')).theme,'corporate');
assert.equal(JSON.parse(store.get('magnus.appearance')).accent,'clay');
fail=false; store.set('magnus.appearance', JSON.stringify({mode:'system',accent:'clay'}));
U.os=()=>({dark:true});U.scope('appliance-c','alice');assert.equal(U.look.mode,'dark');
U.set({followComputer:false});assert.equal(U.prefs().mode,'dark');
U.os=()=>({dark:false});U.apply();assert.equal(U.look.mode,'dark','disabled Follow computer must freeze migrated system mode');
assert.equal(U.prefs().accent,'clay');
assert.ok(events.some(e=>e.type==='magnus-theme'));
console.log('v2: 4 presets × 2 materials, extreme-backdrop contrast, 72 legacy combinations, isolation and storage failure passed');
