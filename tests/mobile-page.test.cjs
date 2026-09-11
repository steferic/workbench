// Run with: node --test tests/mobile-page.test.cjs
// Execute the shipped inline script; mocks only replace browser/network edges.
const {test} = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs');
const vm = require('node:vm');
const {webcrypto} = require('node:crypto');

const source = fs.readFileSync('src/remote/page.rs', 'utf8');
const script = [...source.matchAll(/<script>([\s\S]*?)<\/script>/g)].map(match => match[1]).find(script => script.includes('const token =')).split('\nsetForm(store.get(')[0];
const response = (value = {}, status = 200) => ({ok:status < 400, status,
  text:async () => status < 400 ? JSON.stringify(value) : String(value),
  headers:{get:() => null}});
const turn = () => new Promise(resolve => setImmediate(resolve));

function page(fetch = async () => response({ok:true}), saved = {}) {
  const elements = new Map();
  function element(id) {
    if (!elements.has(id)) elements.set(id, {
      id, value:'', textContent:'', innerHTML:'', hidden:false, disabled:false,
      style:{setProperty(){}}, dataset:{}, scrollHeight:38, scrollTop:0, clientHeight:500,
      classList:{toggle(){}}, listeners:{},
      addEventListener(name, callback) { this.listeners[name] = callback; },
      dispatchEvent(event) { this.listeners[event.type]?.({target:this}); },
      appendChild(){}, setAttribute(){}, removeAttribute(){}, querySelector(){return element('child')},
      querySelectorAll(){return []}, focus(){},
    });
    return elements.get(id);
  }
  const localStorage = {getItem:key => saved[key] ?? null,
    setItem:(key,value) => {saved[key] = value}, removeItem:key => delete saved[key]};
  const document = {getElementById:element, querySelector:element, createElement:element, body:element("body"),
    querySelectorAll:() => [], addEventListener(){}, documentElement:element('root')};
  const context = vm.createContext({document, localStorage, location:{search:'?t=test'},
    URLSearchParams, URL:{createObjectURL:() => 'blob:test', revokeObjectURL(){}},
    navigator:{language:'en-US'}, window:{SpeechRecognition:class {start(){} stop(){}}},
    Event:class {constructor(type){this.type=type}}, fetch, crypto:webcrypto,
    AbortController, setTimeout, clearTimeout, queueMicrotask, innerHeight:844,
    addEventListener(){}, console});
  vm.runInContext(script, context);
  vm.runInContext(`render = () => {}; current = 'A'; attached = draftFor('A').attached;
    data = {instance:'desktop-1', agents:[], projects:[]};`, context);
  const run = code => vm.runInContext(code, context);
  const json = code => JSON.parse(run(`JSON.stringify(${code})`));
  const type = text => {element('msg').value = text; element('msg').dispatchEvent({type:'input'});};
  return {run, json, element, type, context, saved};
}

test('failed sends preserve drafts; retry uses the same ID and clears only after acknowledgment', async () => {
  const requests = [];
  let fail = true;
  const p = page(async (url, options) => {
    requests.push(options);
    return fail ? response('simulated outage', 503) : response({ok:true});
  });
  p.type('keep this draft'); p.run('sendMessage()'); await turn();
  assert.equal(p.element('msg').value, 'keep this draft');
  assert.equal(p.json('outbox')[0].status, 'unknown');
  fail = false; await p.run('deliver(outbox[0].id)');
  assert.equal(requests[0].headers['X-Command-Id'], requests[1].headers['X-Command-Id']);
  assert.equal(p.element('msg').value, '');
  assert.equal(p.json('outbox')[0].status, 'accepted');
  p.run('clearTimeout(noteTimer)');
});

test('double taps submit once, and an acknowledgment preserves a newer draft', async () => {
  let finish, count = 0;
  const p = page(() => {count++; return new Promise(resolve => {finish = resolve});});
  p.type('first'); p.run('sendMessage(); sendMessage()');
  assert.equal(count, 1);
  p.type('a newer draft'); finish(response({ok:true})); await turn();
  assert.equal(p.element('msg').value, 'a newer draft');
  p.run('clearTimeout(noteTimer)');
});

test('drafts and unknown requests survive a page reload', async () => {
  const saved = {};
  const first = page(async () => response('offline',503), saved);
  first.type('survive reload'); first.run('sendMessage()'); await turn();
  const second = page(undefined, saved);
  assert.equal(second.json('draftFor("A")').text, 'survive reload');
  assert.equal(second.json('outbox')[0].id, first.json('outbox')[0].id);
  assert.equal(second.json('outbox')[0].status, 'unknown');
});

test('every file in a delayed batch stays with its original agent', async () => {
  let finish;
  const uploads = [];
  const p = page((url) => {
    uploads.push(url);
    if (uploads.length === 1) return new Promise(resolve => {finish = resolve});
    return Promise.resolve(response({path:'/uploads/A/two.jpg'}));
  });
  p.run('refresh = () => {};');
  const pending = p.element('file').listeners.change({target:{value:'',files:[
    {name:'one.jpg',type:'image/jpeg'}, {name:'two.jpg',type:'image/jpeg'}]}});
  p.run('pick("B")'); finish(response({path:'/uploads/A/one.jpg'})); await pending;
  assert.equal(p.json('draftFor("A").attached').length, 2);
  assert.deepEqual(p.json('draftFor("B").attached'), []);
  assert.ok(uploads.every(url => url.includes('agent=A')));
  p.run('clearTimeout(noteTimer)');
});

test('dictation retains prior utterances and cannot update a different agent', () => {
  const p = page(); p.type('caption');
  p.run(`toggleMic(); recog.onresult({resultIndex:0, results:[[{transcript:'first. '}]]});
    recog.onresult({resultIndex:1, results:[[{transcript:'first. '}],[{transcript:'second.'}]]});`);
  assert.equal(p.element('msg').value, 'caption first. second.');
  p.run('refresh = () => {}; const oldCallback = recog.onresult; pick("B"); oldCallback({resultIndex:0,results:[[{transcript:"late"}]]});');
  assert.equal(p.element('msg').value, '');
});

test('same-sized terminal and message replacements redraw, but unchanged content does not', () => {
  const p = page();
  // Restore the real render function while replacing its DOM reconciliation edge.
  const render = script.slice(script.indexOf('function render()'), script.indexOf('let refreshing = false;'));
  p.run(render);
  p.run(`let paints=0; renderTree=()=>{}; reconcileLog=()=>{paints++};
    data.agents=[{id:'A',project_id:'p',status:'idle',queued:[],messages:[],tail:['old']}];
    data.projects=[{id:'p'}]; render();`);
  assert.equal(p.run('paints'), 1);
  p.run(`data.agents[0].tail=['new']; render(); render();`);
  assert.equal(p.run('paints'), 2);
  p.run(`thread=[{role:'agent',text:'one'}]; render(); thread[0].text='two'; render();`);
  assert.equal(p.run('paints'), 4);
});

test('history remains bounded while its server cursor continues to advance', () => {
  const p = page();
  p.run(`merge({messages:Array.from({length:300},(_,i)=>({role:'agent',text:String(i)})),
    msg_reset:true,msg_total:300,msg_epoch:'one'});`);
  assert.equal(p.run('thread.length'), 200);
  assert.equal(p.run('have'), 300);
  p.run(`merge({messages:[],msg_reset:true,msg_total:0,msg_epoch:'two'});`);
  assert.equal(p.run('thread.length'), 0);
  assert.equal(p.run('have'), 0);
});

test('a response for the previous agent cannot advance the newly selected conversation', async () => {
  let finish;
  const p = page(() => new Promise(resolve => {finish=resolve}));
  p.run('queueMicrotask = () => {};');
  const pending = p.run('refresh()');
  p.run('current="B"; thread=[]; have=0; epoch="";');
  finish(response({agents:[{id:'A',messages:[{role:'agent',text:'old'}],msg_total:1,msg_epoch:'a'}]}));
  await pending;
  assert.equal(p.run('have'), 0);
  assert.deepEqual(p.json('thread'), []);
});

test('status reads continue while a write is waiting for its acknowledgment', async () => {
  let finish;
  const p = page((url, options) => options.method === 'POST'
    ? new Promise(resolve => {finish=resolve})
    : Promise.resolve(response({instance:'desktop-1',projects:[],agents:[],at:123})));
  const sending = p.run('post("/api/reply",{agent:"A",text:"hello"})');
  await p.run('refresh()');
  assert.equal(p.run('data.at'), 123);
  finish(response({ok:true})); await sending;
});

test('request deadlines abort a stalled connection', async () => {
  const p = page((_url, options) => new Promise((_resolve,reject) => {
    options.signal.addEventListener('abort', () => reject(new Error('aborted')));
  }));
  await assert.rejects(p.run('request("/api/state",{},5)'), /aborted/);
});

test('media cards belong to their agent and keep untrusted filenames as text', () => {
  const p = page();
  p.run(`data.media = [
    {id:'image-1',agent:'A',name:'screen <img onerror=oops>.png',kind:'image'},
    {id:'video-1',agent:'A',name:'demo.mp4',kind:'video',poster:true},
    {id:'private',agent:'B',name:'another agent.png',kind:'image'}];`);
  const html = p.run('mediaHtml("A")');
  assert.match(html, /\/media\/image-1\/file\?t=test/);
  assert.match(html, /\/media\/video-1\/poster\?t=test/);
  assert.match(html, /&lt;img onerror=oops&gt;/);
  assert.match(html, /rel="noopener noreferrer"/);
  assert.doesNotMatch(html, /private|another agent|<img onerror/);
});
