import assert from 'node:assert/strict';
import {createRequire} from 'node:module';
import {join,dirname} from 'node:path';
import {fileURLToPath} from 'node:url';
import {mkdirSync} from 'node:fs';
const root=join(dirname(fileURLToPath(import.meta.url)),'../..');
const {chromium}=createRequire(import.meta.url)(join(root,'browser/node_modules/playwright-core'));
const browser=await chromium.launch({executablePath:process.env.MAGNUS_CHROMIUM,args:['--no-sandbox','--disable-dev-shm-usage']});
const page=await browser.newPage({viewport:{width:1440,height:900}}), errors=[];page.on('pageerror',e=>errors.push(e.message));
const until=async(fn,arg)=>{const end=Date.now()+15000;while(Date.now()<end){if(await page.evaluate(fn,arg))return;await page.waitForTimeout(40);}throw Error('UI condition timed out');};
const state=()=>page.evaluate(()=>({nav:document.body.dataset.navigation,mode:document.body.dataset.workspace,region:document.body.dataset.region,width:document.querySelector('#main').getBoundingClientRect().width,work:document.querySelector('#detail').getBoundingClientRect().width,stored:Theme.prefs().conversationWidthPx,overflow:document.documentElement.scrollWidth>innerWidth}));
const select=async region=>{const b=page.locator(`button[data-region=${region}]`);if(await b.isVisible())await b.click();};
try {
 await page.goto(process.env.MAGNUS_URL);await page.getByLabel('Username').fill('alice');await page.getByLabel('Password').fill(process.env.MAGNUS_PASSWORD);await page.getByRole('button',{name:'Sign in',exact:true}).click();await page.locator('#msg').waitFor();
 await page.locator('#msg').fill('Keep this composer text');
 await page.getByRole('link',{name:'Files',exact:true}).click();await page.locator('[data-panel=files]').waitFor();
 let s=await state();assert.equal(s.mode,'split');assert.equal(s.width,420);assert.ok(s.work>=560);assert.equal(s.nav,'expanded');
 await page.getByRole('button',{name:'Collapse navigation',exact:true}).click();assert.equal((await state()).work,s.work+164);
 await page.getByRole('button',{name:'Projects and conversations',exact:true}).click();await page.getByRole('dialog',{name:'Projects and conversations'}).waitFor();await page.keyboard.press('Escape');assert.equal(await page.getByRole('button',{name:'Projects and conversations',exact:true}).getAttribute('aria-expanded'),'false');
 await page.getByRole('button',{name:'Expand navigation',exact:true}).click();assert.equal((await state()).nav,'expanded');
 const sep=page.getByRole('separator',{name:'Conversation width'});await sep.focus();await page.keyboard.press('ArrowLeft');assert.equal((await state()).width,436);await page.keyboard.press('Shift+ArrowRight');assert.equal((await state()).width,388);await page.keyboard.press('End');assert.equal((await state()).width,560);await page.keyboard.press('Home');assert.equal((await state()).width,360);
 await page.evaluate(()=>Theme.set({conversationWidthPx:420}));
 // Available widths with compact navigation: 936/995/996.
 await page.getByRole('button',{name:'Collapse navigation',exact:true}).click();
 for(const [available,want] of [[936,360],[995,419],[996,420]]) {await page.setViewportSize({width:available+64,height:768});await until(w=>document.querySelector('#main').clientWidth===w,want);s=await state();assert.equal(s.width,want);assert.equal(s.stored,420);assert.ok(s.work>=560);}
 await page.setViewportSize({width:1440,height:900});await until(()=>document.querySelector('#main').clientWidth===420);
 // Expansion persists through responsive change; selecting conversation leaves work open.
 await page.getByRole('button',{name:'Expand work',exact:true}).click();assert.equal((await state()).mode,'single');await select('conversation');assert.equal(await page.locator('#msg').inputValue(),'Keep this composer text');await select('work');
 await page.setViewportSize({width:1024,height:768});assert.equal((await state()).mode,'single');await page.getByRole('button',{name:'Restore split view',exact:true}).click();assert.equal((await state()).mode,'split');
 await page.getByRole('button',{name:'Expand work',exact:true}).click();await page.setViewportSize({width:900,height:720});await page.getByRole('button',{name:'Restore split view',exact:true}).click();assert.equal((await state()).mode,'single');
 await select('conversation');assert.equal(await page.locator('#msg').inputValue(),'Keep this composer text');
 for(const [width,height] of [[1920,1080],[1440,900],[1280,720],[1024,768],[390,844],[320,640]]) {
  await page.setViewportSize({width,height});await page.waitForTimeout(50);s=await state();assert.equal(s.overflow,false,`${width} page overflow`);
  if(width<760){assert.equal(s.nav,'drawer');assert.equal(s.mode,'single');await page.getByRole('button',{name:'Open navigation',exact:true}).click();const drawer=page.getByRole('dialog',{name:'Navigation',exact:true});await drawer.waitFor();await page.getByRole('link',{name:/^Needs you/}).focus();await page.keyboard.press('Escape');await until(()=>!document.querySelector('#navigation-drawer').open);assert.equal(await page.evaluate(()=>document.activeElement?.getAttribute('aria-label')),'Open navigation');}
  await select('conversation');assert.equal(await page.locator('#msg').inputValue(),'Keep this composer text');const box=await page.locator('#send').boundingBox();assert.ok(box.y>=0 && box.y+box.height<=height,`${width} composer reachable`);
  if(process.env.MAGNUS_SHOTS){mkdirSync(process.env.MAGNUS_SHOTS,{recursive:true});await page.screenshot({path:join(process.env.MAGNUS_SHOTS,`workspace-${width}.png`)});}
 }
 await page.setViewportSize({width:1440,height:900});await page.getByRole('button',{name:'Close work',exact:true}).click();assert.equal((await state()).mode,'conversation');assert.equal(await page.locator('#msg').inputValue(),'Keep this composer text');
 await page.getByRole('link',{name:'Activity',exact:true}).click();await page.getByRole('navigation',{name:'Activity views'}).waitFor();await page.getByRole('link',{name:'Schedules',exact:true}).click();await page.locator('[data-panel=schedules]').waitFor();
 await page.getByRole('link',{name:'Apps',exact:true}).click();await page.getByRole('heading',{name:'Apps',exact:true}).waitFor();await page.getByPlaceholder('Search apps and capabilities').fill('calendar');await page.getByRole('link',{name:'Ask Magnus to find an app'}).click();await until(()=>document.querySelector('#msg')?.value.includes('calendar'));assert.equal(await page.locator('#thread [data-entry]').count(),0);
 assert.deepEqual(errors,[]);console.log('responsive shell, current resize bounds, rail, drawer, expansion, composer, Activity and literal Apps route passed');
} finally {await browser.close();}
