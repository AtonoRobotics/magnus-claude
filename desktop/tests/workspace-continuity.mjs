import assert from 'node:assert/strict';
import {createRequire} from 'node:module';
import {join,dirname} from 'node:path';
import {fileURLToPath} from 'node:url';
const root=join(dirname(fileURLToPath(import.meta.url)),'../..');
const {chromium}=createRequire(import.meta.url)(join(root,'browser/node_modules/playwright-core'));
const browser=await chromium.launch({executablePath:process.env.MAGNUS_CHROMIUM,args:['--no-sandbox','--disable-dev-shm-usage']});
const context=await browser.newContext();const page=await context.newPage();const errors=[];page.on('pageerror',e=>errors.push(e.message));
const base=process.env.MAGNUS_URL;
const signIn=async person=>{await page.getByLabel('Username').fill(person);await page.getByLabel('Password').fill(process.env.MAGNUS_PASSWORD);await page.getByRole('button',{name:'Sign in',exact:true}).click();await page.locator('#msg').waitFor();};
const until=async(fn,arg)=>{const end=Date.now()+30000;while(Date.now()<end){if(await page.evaluate(fn,arg))return;await page.waitForTimeout(40);}throw Error('UI condition timed out');};
const draft=()=>page.evaluate(()=>App.draftRecord.id);
const newDraft=async()=>{const old=await draft();await page.getByRole('link',{name:'New conversation',exact:true}).click();await until(old=>App.draftRecord?.id!==old,old);return draft();};
const openDraft=async id=>{await page.locator(`nav.side a[href="#/draft/${id}"]`).click();await until(id=>App.draftRecord?.id===id,id);};
try {
 await page.goto(base);await signIn('alice');
 const first=await draft();await page.locator('#msg').fill('First retained draft');
 let releaseUpload,uploaded;const uploadHeld=new Promise(resolve=>releaseUpload=resolve);
 await page.route('**/v1/files',async route=>{if(route.request().method()!=='POST')return route.continue();const response=await route.fetch();uploaded=await response.json();await uploadHeld;await route.fulfill({response});});
 await page.locator('#say input[data-attach]').setInputFiles({name:'notes.txt',mimeType:'text/plain',buffer:Buffer.from('shared file bytes')});
 await page.locator('.attachment-chip').getByText('Uploading…').waitFor();
 await page.getByRole('link',{name:'Settings',exact:true}).click();releaseUpload();
 await until(id=>MessageDrafts.get(id)?.files[0]?.state==='ready',first);
 await openDraft(first);assert.equal(await page.locator('#msg').inputValue(),'First retained draft');await page.locator('.attachment-chip').getByText('Ready',{exact:true}).waitFor();
 await page.unroute('**/v1/files');
 const second=await newDraft();assert.notEqual(first,second);
 await page.locator('#msg').fill('Second retained draft');await page.reload();await page.locator('#msg').waitFor();
 assert.equal(await page.locator('#msg').inputValue(),'Second retained draft');await openDraft(first);assert.equal(await page.locator('#msg').inputValue(),'First retained draft');
 assert.equal(await page.locator('.attachment-chip').count(),1);
 await page.getByRole('button',{name:'Remove notes.txt from message'}).click();assert.equal(await page.locator('.attachment-chip').count(),0);
 assert.equal((await context.request.get(`${base}/v1/files/${uploaded.id}/info`)).status(),200,'removing attachment must not delete shared file');
 // Opening and Back/close preserve the selected draft and use deliberate focus.
 await page.getByRole('link',{name:'Files',exact:true}).click();await page.locator('#detail [data-panel=files]').waitFor();
 await page.locator(`#detail [data-open="file:${uploaded.id}"]`).click();await page.locator('#detail [data-panel=file]').waitFor();
 await until(()=>document.activeElement?.tagName==='H2' && !!document.activeElement.closest('#detail'));
 assert.equal(await page.locator('#msg').inputValue(),'First retained draft');
 await page.goBack();await page.locator('#detail [data-panel=files]').waitFor();
 await page.locator('#detail [data-act=close-panel]').click();
 assert.ok(await page.evaluate(()=>document.activeElement===document.querySelector('#conv-head h1')));
 // A removed pending upload may finish, but must never reattach itself.
 let releaseLate,lateFile;const lateHeld=new Promise(resolve=>releaseLate=resolve);
 await page.route('**/v1/files',async route=>{if(route.request().method()!=='POST')return route.continue();const response=await route.fetch();lateFile=await response.json();await lateHeld;await route.fulfill({response});});
 await page.locator('#say input[data-attach]').setInputFiles({name:'late.txt',mimeType:'text/plain',buffer:Buffer.from('late completion')});
 await page.locator('.attachment-chip').getByText('Uploading…').waitFor();await page.getByRole('button',{name:'Remove late.txt from message'}).click();const lateFinished=page.waitForEvent('requestfinished',{predicate:r=>r.method()==='POST' && new URL(r.url()).pathname==='/v1/files'});releaseLate();await lateFinished;
 await until(()=>MessageDrafts.all().every(r=>r.files.every(f=>f.name!=='late.txt')));await page.unroute('**/v1/files');

 // Restart during an unfinished upload retains metadata, never raw bytes.
 const interrupted=await newDraft();await page.locator('#msg').fill('Interrupted upload draft');
 let endInterrupted;const interruptedHeld=new Promise(resolve=>endInterrupted=resolve);
 await page.route('**/v1/files',async route=>{if(route.request().method()!=='POST')return route.continue();await interruptedHeld;try{await route.abort('failed');}catch{/* reload already aborted it */}});
 await page.locator('#say input[data-attach]').setInputFiles({name:'interrupted.txt',mimeType:'text/plain',buffer:Buffer.from('PRIVATE-UPLOAD-BYTES')});
 await page.locator('.attachment-chip').getByText('Uploading…').waitFor();await page.reload();
 await page.locator('.attachment-chip').getByText('Failed',{exact:true}).waitFor();assert.equal(await page.locator('#msg').inputValue(),'Interrupted upload draft');
 assert.equal(await page.evaluate(()=>JSON.stringify(localStorage).includes('PRIVATE-UPLOAD-BYTES')),false);
 endInterrupted();await page.unroute('**/v1/files');await openDraft(first);
 // IME Enter must leave the draft and perform no send.
 const before=(await(await context.request.get(`${base}/v1/chat`)).json()).threads.length;
 await page.locator('#msg').dispatchEvent('keydown',{key:'Enter',code:'Enter',isComposing:true});
 assert.equal((await(await context.request.get(`${base}/v1/chat`)).json()).threads.length,before);
 assert.equal(await page.locator('#msg').inputValue(),'First retained draft');
 // The server accepts first send, while the user continues typing before ACK.
 let releaseSend,accepted;const sendHeld=new Promise(resolve=>releaseSend=resolve);
 await page.route('**/v1/chat',async route=>{if(route.request().method()!=='POST')return route.continue();const response=await route.fetch();accepted=await response.json();await sendHeld;await route.fulfill({response});});
 await page.locator('#send').click();await until(()=>document.querySelector('#msg').value==='');
 await page.locator('#msg').fill('Newer text while sending');releaseSend();
 await page.waitForURL(/#\/chat\//);await until(()=>document.querySelector('#msg')?.value==='Newer text while sending');
 assert.equal(await page.locator(`nav.side a[href="#/draft/${first}"]`).count(),0,'ACK replaces local new-conversation entry');
 await page.reload();await until(()=>document.querySelector('#msg')?.value==='Newer text while sending');
 await page.unroute('**/v1/chat');
 // An actual accepted message whose HTTP response is lost stays unconfirmed.
 const uncertain=await newDraft();await page.locator('#msg').fill('Response will be lost');
 let lostThread,postCount=0;
 await page.route('**/v1/chat',async route=>{if(route.request().method()!=='POST')return route.continue();postCount++;const response=await route.fetch();lostThread=(await response.json()).thread;await route.abort('failed');});
 await page.locator('#send').click();await page.locator('#submitted').getByText('Send outcome not confirmed',{exact:true}).waitFor();
 await page.locator('#msg').fill('Newer edits after lost response');await page.reload();
 await page.locator('#submitted').getByText('Send outcome not confirmed',{exact:true}).waitFor();assert.equal(await page.locator('#msg').inputValue(),'Newer edits after lost response');assert.equal(postCount,1);
 const acceptedLost=(await(await context.request.get(`${base}/v1/chat/${lostThread}`)).json()).entries.filter(e=>e.kind==='message');assert.equal(acceptedLost.length,1);
 await page.unroute('**/v1/chat');
 // Definite server validation rejection keeps submitted and newer text separate.
 const rejectedId=await newDraft();await page.locator('#msg').fill('Rejected submitted text');
 const validModel=await page.locator('#say [data-model]').inputValue();
 await page.locator('#say [data-model]').evaluate(select=>{const option=new Option('Removed model',JSON.stringify(['missing-adapter','removed-model']));select.append(option);select.value=option.value;});
 let releaseRejected;const rejectedHeld=new Promise(resolve=>releaseRejected=resolve);
 await page.route('**/v1/chat',async route=>{if(route.request().method()!=='POST')return route.continue();const response=await route.fetch();assert.equal(response.status(),400);await rejectedHeld;await route.fulfill({response});});
 await page.locator('#send').click();await until(()=>document.querySelector('#msg').value==='');await page.locator('#msg').fill('Newer edits after rejection');releaseRejected();
 await page.locator('#submitted').getByText('Message was not accepted',{exact:true}).waitFor();assert.equal(await page.locator('#msg').inputValue(),'Newer edits after rejection');
 await page.unroute('**/v1/chat');await page.getByRole('button',{name:'Edit rejected message'}).click();await until(()=>document.querySelector('#msg')?.value==='Rejected submitted text');
 assert.equal(await page.evaluate(id=>MessageDrafts.get(id).text,rejectedId),'Newer edits after rejection');await page.locator('#say [data-model]').selectOption(validModel);
 // Scroll anchoring uses an entry identity; new data never forces latest.
 for(let i=0;i<20;i++){const response=await page.evaluate(body=>Magnus.api('POST','/v1/chat',body),{thread:accepted.thread,text:`Anchor entry ${i}`});assert.equal(response.thread,accepted.thread);}
 await page.goto(`${base}/#/chat/${accepted.thread}`);await page.locator('.entry[data-kind=message]').nth(20).waitFor();
 await page.locator('#thread').evaluate(el=>{el.scrollTop=120;});await page.waitForTimeout(350);
 const anchor=()=>page.locator('#thread').evaluate(el=>{const top=el.getBoundingClientRect().top;const n=[...el.querySelectorAll('[data-entry]')].find(n=>n.getBoundingClientRect().bottom>top);return{id:n.dataset.entry,offset:n.getBoundingClientRect().top-top,bottom:el.scrollHeight-el.scrollTop-el.clientHeight};});
 const previous=await anchor();assert.ok(previous.bottom>80);
 const response=await page.evaluate(body=>Magnus.api('POST','/v1/chat',body),{thread:accepted.thread,text:'Activity while reading above'});assert.equal(response.thread,accepted.thread);
 await page.locator('.entry[data-kind=message]').nth(21).waitFor();const after=await anchor();assert.equal(after.id,previous.id);assert.ok(Math.abs(after.offset-previous.offset)<3);
 await page.getByRole('button',{name:'New activity · Jump to latest'}).click();assert.ok((await anchor()).bottom<=80);

 // Unavailable storage keeps the current draft through ordinary navigation.
 await page.evaluate(()=>{window.originalStorageSet=Storage.prototype.setItem;Storage.prototype.setItem=function(){throw new Error('test storage unavailable');};});
 await page.locator('#msg').fill('Unsaved text stays in memory');await page.getByText('Draft not saved on this device',{exact:true}).waitFor();
 await page.getByRole('link',{name:'Settings',exact:true}).click();
 await page.locator(`nav.side a[href="#/chat/${accepted.thread}"]`).click();await until(()=>document.querySelector('#msg')?.value==='Unsaved text stays in memory');
 await page.evaluate(()=>{Storage.prototype.setItem=window.originalStorageSet;delete window.originalStorageSet;MessageDrafts.flush();});
 // Explicit sign-out clears Alice's message journal; Bob sees none of it.
 await page.getByRole('button',{name:'Sign out',exact:true}).click();await signIn('bob');
 assert.equal(await page.evaluate(()=>MessageDrafts.all().some(r=>r.text.includes('retained')||r.pending.some(p=>p.body.text==='Response will be lost'))),false);
 await page.getByRole('button',{name:'Sign out',exact:true}).click();await signIn('alice');
 assert.equal(await page.evaluate(id=>!!MessageDrafts.get(id),uncertain),false);
 assert.deepEqual(errors,[]);console.log('browser: distinct drafts/restart, uploads across navigation, nondeleting removal, IME, newer edits after ACK/lost response, anchors, work focus/history, storage failure and scoped sign-out passed');
} finally {await browser.close();}
