import assert from 'node:assert/strict';
import {createRequire} from 'node:module';import {join,dirname} from 'node:path';import {fileURLToPath} from 'node:url';
const root=join(dirname(fileURLToPath(import.meta.url)),'../..');const {chromium}=createRequire(import.meta.url)(join(root,'browser/node_modules/playwright-core'));
const browser=await chromium.launch({executablePath:process.env.MAGNUS_CHROMIUM,args:['--no-sandbox','--disable-dev-shm-usage']});const page=await browser.newPage(),errors=[];page.on('pageerror',e=>errors.push(e.message));
const until=async(fn,arg)=>{for(const end=Date.now()+15000;Date.now()<end;await page.waitForTimeout(40))if(await page.evaluate(fn,arg))return;throw Error('UI condition timed out');};
try{
 await page.goto(process.env.MAGNUS_URL);await page.getByLabel('Username').fill('alice');await page.getByLabel('Password').fill(process.env.MAGNUS_PASSWORD);await page.getByRole('button',{name:'Sign in',exact:true}).click();await page.locator('#msg').waitFor();
 await page.evaluate(()=>{background.forEach(clearInterval);background=[];});
 const missingSchedule=await page.evaluate(()=>scheduleLine({state:'confirmed',schedule:'not-in-current-view',title:'Known schedule'}));assert.ok(missingSchedule.includes('Details unavailable'));assert.ok(!missingSchedule.includes('Cancelled') && !missingSchedule.includes('no longer runs'),'missing collection data cannot establish cancellation');
 // A delayed catalog response from an older view cannot overwrite shared catalog state.
 let releaseCatalog,catalogStarted=false,catalogCount=0;const catalogHeld=new Promise(r=>releaseCatalog=r);
 await page.route('**/v1/connectors',async r=>{if(r.request().method()!=='GET')return r.continue();const response=await r.fetch();catalogCount=(await response.json()).connectors.length;catalogStarted=true;await catalogHeld;await r.fulfill({response});});
 const oldCatalog=page.evaluate(()=>showLibrary(document.querySelector('#main')).then(()=>false,e=>!!e.cancelled));oldCatalog.catch(()=>{});for(let n=0;n<375 && !catalogStarted;n++)await page.waitForTimeout(40);assert.ok(catalogStarted,'catalog request captured');assert.ok(catalogCount>0,'use actual configured catalog data');
 await page.getByRole('link',{name:'Settings',exact:true}).click();await page.getByRole('heading',{name:'Settings',exact:true,level:1}).waitFor();releaseCatalog();assert.equal(await oldCatalog,true);assert.equal(await page.evaluate(()=>Lib.all),undefined);await page.unroute('**/v1/connectors');
 await page.getByRole('link',{name:'New conversation',exact:true}).click();await page.locator('#msg').waitFor();await page.evaluate(()=>{background.forEach(clearInterval);background=[];});
 // A completed real task leads with its result and keeps optional evidence/feedback compact.
 await page.evaluate(id=>openPanel(`task:${id}`),process.env.MAGNUS_RESULT_TASK);await page.frameLocator('#pview').getByText('Actual stored deliverable').waitFor();assert.equal(await page.locator('.verification-detail').getAttribute('open'),null);assert.equal(await page.getByRole('button',{name:'Right',exact:true}).isVisible(),false);await page.locator('[data-work-feedback] > summary').click();await page.getByRole('button',{name:'Right',exact:true}).click();await until(async id=>(await Magnus.api('GET',`/v1/tasks/${id}`)).verdict?.verdict==='right',process.env.MAGNUS_RESULT_TASK);await page.evaluate(()=>closePanel());
 await page.evaluate(async()=>{await Magnus.api('POST','/v1/projects',{name:'Retained project'});await refresh();});
 assert.equal(await page.evaluate(()=>App.projects.length),1);
 await page.route('**/v1/projects',r=>r.request().method()==='GET' ? r.fulfill({status:503,contentType:'application/json',body:'{"error":"unavailable"}'}) : r.continue());
 await page.evaluate(()=>refresh());await page.locator('#connection-status').getByText(/Projects may be stale/).waitFor();assert.equal(await page.evaluate(()=>App.projects[0].name),'Retained project');
 const newThread=await page.evaluate(async()=>{const r=await Magnus.api('POST','/v1/chat',{text:'Actual new conversation'});await refresh();return r.thread;});assert.ok(await page.evaluate(id=>App.threads.some(t=>t.thread===id),newThread),'healthy conversations continue updating');const cachedThreads=await page.evaluate(()=>App.threads.map(t=>t.thread));
 const announcement=await page.locator('#connection-announcement').textContent();await page.evaluate(()=>refresh());assert.equal(await page.locator('#connection-announcement').textContent(),announcement,'unchanged failures are not announced again');
 await page.unroute('**/v1/projects');await page.evaluate(()=>refresh());await until(()=>document.querySelector('#connection-status').hidden);
 // An older successful response must not replace newer authoritative objects.
 let release,started=false;const held=new Promise(r=>release=r);
 await page.route('**/v1/projects',async r=>{if(r.request().method()!=='GET' || started)return r.continue();started=true;const response=await r.fetch();await held;await r.fulfill({response});});
 const old=page.evaluate(()=>refresh());await until(()=>true);for(let n=0;n<375 && !started;n++)await page.waitForTimeout(40);assert.ok(started,'project request captured');
 await page.evaluate(async()=>{await Magnus.api('POST','/v1/projects',{name:'New authoritative project'});await refresh();});assert.equal(await page.evaluate(()=>App.projects.length),2);release();await old;assert.equal(await page.evaluate(()=>App.projects.length),2);
 await page.unroute('**/v1/projects');
 const failure='**/v1/{tasks,schedules,chat,projects,health,inbox,versions}';await page.route(failure,r=>r.request().method()==='GET' ? r.abort('failed') : r.continue());await page.evaluate(()=>refresh());
 assert.equal(await page.evaluate(()=>App.projects.length),2);assert.deepEqual(await page.evaluate(()=>App.threads.map(t=>t.thread)),cachedThreads);await page.locator('#connection-status').getByText(/Tasks may be stale/).waitFor();await page.unroute(failure);await page.evaluate(()=>refresh());await until(()=>document.querySelector('#connection-status').hidden);
 // A failed work refresh keeps the actual composed file available.
 const file=await page.evaluate(async()=>Magnus.api('POST','/v1/files',{name:'cached.txt',media_type:'text/plain',data_b64:btoa('Readable cached work')}));
 await page.evaluate(id=>openPanel(`file:${id}`),file.id);await page.frameLocator('#fview').getByText('Readable cached work').waitFor();
 await page.route(`**/v1/files/${file.id}/info`,r=>r.fulfill({status:503,contentType:'application/json',body:'{"error":"unavailable"}'}));
 await page.evaluate(()=>drawPanel());await page.frameLocator('#fview').getByText('Readable cached work').waitFor();await page.locator('#connection-status').getByText(/Work may be stale/).waitFor();
 await page.unroute(`**/v1/files/${file.id}/info`);await page.evaluate(()=>drawPanel());await until(()=>document.querySelector('#connection-status').hidden);
 // A session failure invokes the existing sign-in flow and invalidates all outstanding resource replies.
 await page.route('**/v1/inbox',r=>r.fulfill({status:401,contentType:'application/json',body:'{"error":"session expired"}'}));await page.evaluate(()=>refresh());await page.getByLabel('Username').waitFor();assert.equal(await page.locator('#si').count(),1);
 assert.deepEqual(errors,[]);console.log('partial/all resource failures retain actual data; healthy updates, latest response and recovery passed');
}finally{await browser.close();}
