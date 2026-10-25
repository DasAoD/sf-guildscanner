const $=id=>document.getElementById(id);
const t=key=>(window.I18N.de&&window.I18N.de[key])||key;

let results=[], ownGuild=null;
let sortKey='rank', sortDir='asc';
let hofResults=[];
let hofSortKey='rank', hofSortDir='asc';
let hasScanData=false, scanDate='';
// Namen der per Checkbox ausgewählten Gilden, getrennt je Tabelle (Ergebnisse =
// rBody, Ehrenhalle = hofBody). Bleibt über renderTable()-Neuaufbauten bestehen
// (z.B. nach applyFilter() nach Simulationsende), damit die Häkchen nicht bei
// jedem Neurendern verschwinden.
let selected={rBody:new Set(),hofBody:new Set()};

const PAGES=['character','overview','scan','results','hof','accounts','users','settings','help'];
// Seiten, die einen gewählten Charakter (und damit eine Gilde) brauchen
const NEEDS_GUILD=['overview','scan','results','hof'];
const MUSH_ICON='<svg class="icon mush"><use href="#i-mush"/></svg>';
let lastMushrooms='0';

// Pilz-Auswahl und Start-Button pro Gildenzeile
function rowActionsHtml(name){
  const opts=[0,1,2,3].map(n=>`<option value="${n}"${String(n)===lastMushrooms?' selected':''}>${n}</option>`).join('');
  return `<span class="rowact"><select class="rowMush" title="Pilze im Katapult">${opts}</select><button class="btn sec small" type="button" data-g="${esc(name)}" onclick="doSimulateRow(this)" title="Nur diese Gilde mit der gewählten Pilzzahl simulieren"><svg class="icon"><use href="#i-play"/></svg>Sim</button></span>`;
}
let view='boot';

document.querySelectorAll('[data-i18n]').forEach(el=>{el.textContent=t(el.dataset.i18n);});

// ── ANSICHTEN / NAVIGATION ─────────────────────────────────
function route(){
  $('authView').classList.toggle('hidden',view!=='auth');
  $('appView').classList.toggle('hidden',view!=='app');
  if(view!=='app')return;
  let page=(location.hash||'#overview').slice(1);
  if(!PAGES.includes(page))page='overview';
  if(page==='users'&&!(authUser&&authUser.is_admin))page='character';
  if(NEEDS_GUILD.includes(page)&&!ownGuild)page='character';
  document.querySelectorAll('.page').forEach(p=>p.classList.toggle('show',p.id==='p-'+page));
  document.querySelectorAll('.nbtn[data-page]').forEach(b=>b.classList.toggle('active',b.dataset.page===page));
  $('side').classList.remove('open');
  if(page==='users')loadUsers();
  if(page==='character')loadCharacterPage();
  if(page==='accounts')loadAccounts();
  window.scrollTo(0,0);
}
function hostOf(url){
  try{return new URL(url).hostname;}catch(e){return url;}
}

// Angemeldeter Benutzer, aktiver Charakter und Gilde oben in der Seitenleiste
function updateWho(){
  if(!authUser){$('who').textContent=''; return;}
  let html=`Angemeldet als <b>${esc(authUser.name)}</b>${authUser.is_admin?' (Admin)':''}`;
  if(activeChar&&ownGuild){
    html+=`<br><span class="chr" title="${esc(activeChar.server)}">${esc(activeChar.name)}</span> <span class="gld">${esc(ownGuild.name)}</span>`;
  }
  $('who').innerHTML=html;
}

function goPage(page){
  if(location.hash==='#'+page)route(); else location.hash='#'+page;
}
window.addEventListener('hashchange',route);
$('menu').addEventListener('click',()=>$('side').classList.toggle('open'));

async function api(ep,data){
  const opts=data!==undefined
    ?{method:'POST',headers:{'Content-Type':'application/json'},body:JSON.stringify(data||{})}
    :{};
  const r=await fetch('/api/'+ep,opts);
  if(r.status===401&&ep!=='auth/login'){showAuth('login');return {success:false,error:'Nicht angemeldet'};}
  return r.json();
}

function fmt1(n){return (typeof n==='number'?n:0).toLocaleString('de',{minimumFractionDigits:1,maximumFractionDigits:1});}

// ── LOGIN ──────────────────────────────────────────────────
async function doLogin(){
  const u=$('user').value.trim(), p=$('pass').value;
  if(!u||!p)return;
  $('loginBtn').disabled=true; $('loginErr').textContent='';
  try{
    const r=await api('login',{username:u,password:p});
    if(!r.success){$('loginErr').textContent=r.error;return;}
    showChars(r.data);
  }catch(e){$('loginErr').textContent='Verbindungsfehler';}
  finally{$('loginBtn').disabled=false;}
}
$('pass').addEventListener('keydown',e=>{if(e.key==='Enter')doLogin();});

// ── CHARACTER ──────────────────────────────────────────────
function showSso(){
  $('ssoCard').classList.remove('hidden');
  $('charCard').classList.add('hidden');
}

// Charakterliste aus den gespeicherten Konten; der einmalige Login ohne Speichern
// bleibt als Rückfall (aufgeklappt, solange noch kein Konto hinterlegt ist).
async function loadCharacterPage(){
  let stored=[];
  $('charErr').textContent='';
  try{
    const r=await api('accounts/characters');
    if(r.success)stored=r.data; else $('charErr').textContent='Gespeicherte Konten konnten nicht gelesen werden: '+r.error;
  }catch(e){/* ohne Antwort gibt es nur den manuellen Login */}
  renderStored(stored);

  let manual=null;
  try{
    const st=await api('status');
    if(st.success&&st.data.logged_in&&!st.data.account_id){
      const r=await api('characters');
      if(r.success)manual=r.data;
    }
  }catch(e){/* ohne Antwort bleibt das Anmeldeformular */}
  if(manual)showChars(manual); else showSso();
  $('manualBox').open=stored.length===0;
}

let activeChar=null;
let charsList=[];

function showChars(chars){
  charsList=chars;
  $('ssoCard').classList.add('hidden');
  $('charCard').classList.remove('hidden');
  const l=$('charList'); l.innerHTML='';
  chars.forEach(c=>{
    const b=document.createElement('button');
    b.className='btn';
    try{b.textContent=`${c.name} (${new URL(c.server).hostname})`;}catch(e){b.textContent=c.name;}
    b.onclick=()=>selectChar(c.index);
    l.appendChild(b);
  });
}

function resetResults(){
  results=[]; hofResults=[]; selected={rBody:new Set(),hofBody:new Set()};
  hasScanData=false; scanDate='';
  $('lastScanLoad').classList.add('hidden');
  $('lastScanInfo').textContent='Noch kein Scan für diesen Charakter.';
  $('resContent').classList.add('hidden'); $('resEmpty').classList.remove('hidden');
  $('hofContent').classList.add('hidden'); $('hofEmpty').classList.remove('hidden');
  $('rCount').textContent=''; $('hofCount').textContent='';
  $('navResCount').hidden=true; $('navResCount').textContent='';
  $('navHofCount').hidden=true; $('navHofCount').textContent='';
  $('rBody').innerHTML=''; $('hofBody').innerHTML='';
  $('progressArea').classList.add('hidden'); $('pMsg').textContent='';
  simUi(el=>el.classList.add('hidden'),'simProgress'); simUi(el=>{el.textContent='';},'simMsg');
  updateSelInfo();
}

// Simulationsanzeige gibt es auf der Ergebnisse- und der Ehrenhalle-Seite;
// beide werden gemeinsam bedient.
function simUi(fn,cls){document.querySelectorAll('.'+cls).forEach(fn);}

function applyScanStatus(st){
  if(!st?.has_scan_data)return;
  hasScanData=true; scanDate=st.scan_date||'';
  $('lastScanInfo').textContent=`vom ${scanDate}`;
  $('lastScanLoad').classList.remove('hidden');
}

async function selectChar(idx){
  $('charList').querySelectorAll('button').forEach(b=>b.disabled=true);
  try{
    const r=await api('select-character',{index:idx});
    if(!r.success){alert(r.error);return;}
    // Stand eines vorher gewählten Charakters (bei Charakterwechsel)
    // nicht mit rüberschleppen.
    resetResults();
    const c=charsList.find(x=>x.index===idx);
    activeChar=c?{name:c.name,server:hostOf(c.server)}:null;
    showGuild(r.data);
    goPage('overview');
    const st=await api('status');
    applyScanStatus(st.data);
  }catch(e){alert('Fehler: '+e.message);}
  finally{$('charList').querySelectorAll('button').forEach(b=>b.disabled=false);}
}

// ── TEILNAHME AN DER SIMULATION ────────────────────────────
// ready: kampfbereit, planable: online, aber nicht für den Angriff angemeldet
// (per Haken für eine Planung zuschaltbar), offline: länger als 24h weg.
let partState={};
// Live-Gilde, auf die sich die Auswahl bezieht (nicht der Scan-Snapshot aus applyFilter)
let partGuild=null;
function memberKind(m){
  if(m.is_active_24h===false)return 'offline';
  if(m.attack_joined===false)return 'planable';
  return 'ready';
}
function loadPart(g){
  partGuild=g;
  let saved={};
  try{saved=JSON.parse(localStorage.getItem('part:'+g.name))||{};}catch(e){/* ohne Speicher gelten die Vorgaben */}
  partState={};
  g.members.forEach(m=>{
    const k=memberKind(m);
    partState[m.name]=k==='offline'?false:(m.name in saved?!!saved[m.name]:k==='ready');
  });
}
function savePart(){
  try{localStorage.setItem('part:'+partGuild.name,JSON.stringify(partState));}catch(e){/* nur Komfort */}
}
function participation(){
  const include=[], exclude=[];
  (partGuild?.members||[]).forEach(m=>{
    const k=memberKind(m), on=!!partState[m.name];
    if(k==='ready'&&!on)exclude.push(m.name);
    if(k==='planable'&&on)include.push(m.name);
  });
  return {include,exclude};
}
function updatePartInfo(){
  const n=Object.values(partState).filter(Boolean).length;
  const ready=(partGuild?.members||[]).filter(m=>memberKind(m)==='ready').length;
  document.querySelectorAll('.partInfo').forEach(el=>{
    el.innerHTML=`Teilnehmer: ${n} (${ready} kampfbereit) <a href="#overview">ändern</a>`;
  });
}

// ── SPALTEN-UMSCHALTER ─────────────────────────────────────
const colHidden={};
function applyCols(tableId){
  const table=$(tableId); if(!table)return;
  const hidden=colHidden[tableId]||new Set();
  [...table.querySelectorAll('thead th')].forEach((th,i)=>{
    const off=hidden.has(th.dataset.col);
    table.querySelectorAll('tr').forEach(tr=>{if(tr.cells[i])tr.cells[i].classList.toggle('colhide',off);});
  });
}
function initColMenus(){
  document.querySelectorAll('.colmenu').forEach(box=>{
    const tableId=box.dataset.for, table=$(tableId), menu=box.querySelector('.menu');
    let saved=[];
    try{saved=JSON.parse(localStorage.getItem('cols:'+tableId))||[];}catch(e){/* nur Komfort */}
    colHidden[tableId]=new Set(saved);
    [...table.querySelectorAll('thead th')].filter(th=>!('lock' in th.dataset)).forEach(th=>{
      const label=document.createElement('label'); label.className='check';
      const cb=document.createElement('input'); cb.type='checkbox'; cb.checked=!colHidden[tableId].has(th.dataset.col);
      cb.addEventListener('change',()=>{
        if(cb.checked)colHidden[tableId].delete(th.dataset.col); else colHidden[tableId].add(th.dataset.col);
        try{localStorage.setItem('cols:'+tableId,JSON.stringify([...colHidden[tableId]]));}catch(e){/* nur Komfort */}
        applyCols(tableId);
      });
      label.append(cb,th.firstChild.textContent.trim());
      menu.append(label);
    });
    box.querySelector('button').addEventListener('click',e=>{e.stopPropagation(); menu.hidden=!menu.hidden;});
    menu.addEventListener('click',e=>e.stopPropagation());
    applyCols(tableId);
  });
  document.addEventListener('click',()=>document.querySelectorAll('.colmenu .menu').forEach(m=>{m.hidden=true;}));
}

// ── OWN GUILD ──────────────────────────────────────────────
function showGuild(g){
  ownGuild=g;
  $('guildName').textContent=g.name;
  $('ovSub').textContent=`Rang #${g.rank}`;
  updateWho();

  const ownMax=Math.max(...g.members.map(m=>m.level));
  const ownMin=Math.min(...g.members.map(m=>m.level));
  const ownTotal=g.members.reduce((s,m)=>s+m.level,0);
  const ownAvg=(g.member_count>0)?(ownTotal/g.member_count):0;
  $('guildStats').innerHTML=`
    <div class="stat"><b>${g.member_count}</b><span>Mitglieder</span></div>
    <div class="stat" title="Online in den letzten 24 Stunden und für den Angriff angemeldet"><b>${(g.active_member_count??g.member_count)}</b><span>Kampfbereit</span></div>
    <div class="stat"><b>${ownMax}</b><span>Max Level</span></div>
    <div class="stat"><b>${ownMin}</b><span>Min Level</span></div>
    <div class="stat"><b>${fmt1(ownAvg)}</b><span>Ø Level</span></div>
    <div class="stat"><b>#${g.rank}</b><span>Rang</span></div>
    <div class="stat"><b>${g.honor.toLocaleString('de')}</b><span>Ehre</span></div>`;

  loadPart(g);
  const tb=$('ownMembers').querySelector('tbody'); tb.innerHTML='';
  [...g.members].sort((a,b)=>b.level-a.level).forEach(m=>{
    const tr=document.createElement('tr');
    const k=memberKind(m);
    const off=(m.offline_days!=null && m.offline_days>=1)?` <span class="badge b-red" title="offline seit ${m.offline_days} Tagen">off ${m.offline_days}d</span>`:'';
    tr.className=k==='ready'?'':'dim';
    tr.innerHTML=`<td>${esc(m.name)}${off}</td><td>${m.level} ${levelBar(m.level,ownMax)}</td>
      <td><input type="checkbox" data-name="${esc(m.name)}" ${partState[m.name]?'checked':''} ${k==='offline'?'disabled':''}></td>`;
    tr.querySelector('input').addEventListener('change',e=>{
      partState[e.target.dataset.name]=e.target.checked; savePart(); updatePartInfo();
    });
    tb.appendChild(tr);
  });
  updatePartInfo();
}

function levelBar(lvl,max){
  const pct=max>0?Math.round(lvl/max*100):0;
  const clr=pct>80?'var(--gold)':pct>50?'var(--accent)':'var(--muted)';
  return `<span class="level-bar" style="width:${Math.max(pct,3)}px;background:${clr}"></span>`;
}

// ── SCAN ───────────────────────────────────────────────────
async function doScan(){
  const down=parseInt($('downLimit').value||'300');
  const honorUp=$('honorUp').checked;
  const strict=$('strictMode').checked;
  const strictTopN=$('strictTopN').checked;
  const extra=parseInt($('extraUpPages').value||'10');
  const fullUpScan=$('fullUpScan').checked;

  $('scanBtn').disabled=true;
  $('abortBtn').disabled=false;
  $('progressArea').classList.remove('hidden');
  $('pMsg').classList.remove('neg');

  try{
    const r=await api('scan',{down_limit:down,honor_up_scan:honorUp,max_extra_up_pages:extra,strict_mode:strict,strict_topn:strictTopN,full_up_scan:fullUpScan});
    if(!r.success){$('pMsg').textContent=r.error; $('pMsg').classList.add('neg'); $('scanBtn').disabled=false; $('abortBtn').disabled=true; return;}
    pollProgress();
  }catch(e){$('pMsg').textContent='Fehler: '+e.message; $('scanBtn').disabled=false; $('abortBtn').disabled=true;}
}

async function doAbort(){
  $('abortBtn').disabled=true;
  try{
    const r=await api('scan/abort',{});
    if(!r.success){$('pMsg').textContent=r.error; $('pMsg').classList.add('neg'); return;}
    $('pMsg').textContent=r.data||'Abbruch angefordert...';
  }catch(e){$('pMsg').textContent='Fehler: '+e.message; $('pMsg').classList.add('neg');}
}

async function pollProgress(){
  try{
    const r=await api('progress');
    if(!r.success)return;
    const p=r.data;
    const pct=p.total>0?Math.round(p.current/p.total*100):0;

    if(p.phase==='hof'){
      $('pFill').style.width=pct+'%';
      $('pText').textContent=`HoF: ${p.current} / ${p.total} Seiten`;
    }else if(p.phase==='details'){
      $('pFill').style.width=pct+'%';
      $('pText').textContent=`${p.current} / ${p.total} Gilden`;
    }else if(p.phase==='done' || p.phase==='aborted'){
      $('pFill').style.width='100%';
      $('pText').textContent=p.phase==='done'?'Fertig!':'Abgebrochen';
      $('scanBtn').disabled=false;
      $('abortBtn').disabled=true;
      await applyFilter();
      const st=await api('status'); applyScanStatus(st.data);
      $('pMsg').textContent=p.message;
      goPage('results');
      return;
    }

    $('pMsg').textContent=p.message;
    if(p.running) setTimeout(pollProgress, 800);
    else {
      $('scanBtn').disabled=false;
      $('abortBtn').disabled=true;
      if(p.phase==='error') $('pMsg').classList.add('neg');
    }
  }catch(e){setTimeout(pollProgress,2000);}
}

async function loadExisting(){
  await applyFilter();
  goPage('results');
}

// ── FILTER & RESULTS ───────────────────────────────────────
async function applyFilter(){
  const maxM=$('fMembers').value?parseInt($('fMembers').value):null;
  const maxLvl=$('fMaxLvl').value?parseInt($('fMaxLvl').value):null;
  const hideAtk=$('fHideAtk').checked;
  const strictOnly=$('fStrict').checked;

  try{
    const r=await api('results',{max_members:maxM,max_highest_level:maxLvl,hide_attacked:hideAtk,strict_only:strictOnly});
    if(!r.success){alert(r.error);return;}
    results=r.data.guilds;
    ownGuild=r.data.own_guild;
    results.forEach(g=>{ g.avg_level = (g.member_count>0)?(g.total_level/g.member_count):0; });
    if(ownGuild && Array.isArray(ownGuild.members)){
      const sum=ownGuild.members.reduce((s,m)=>s+m.level,0);
      ownGuild.avg_level = (ownGuild.member_count>0)?(sum/ownGuild.member_count):0;
    }
    const strictHint = strictOnly ? ' (Strikt OK)' : '';
    $('rCount').textContent=`${r.data.filtered_count} / ${r.data.total_attackable} Kandidaten${strictHint} | Scan: ${r.data.scanned_at}`;
    $('resEmpty').classList.add('hidden'); $('resContent').classList.remove('hidden');
    $('navResCount').textContent=r.data.filtered_count; $('navResCount').hidden=false;
    renderTable();

    hofResults=r.data.hof_guilds||[];
    const hasHof=hofResults.length>0;
    $('hofEmpty').classList.toggle('hidden',hasHof); $('hofContent').classList.toggle('hidden',!hasHof);
    $('navHofCount').textContent=hofResults.length; $('navHofCount').hidden=!hasHof;
    renderHofTable();
  }catch(e){alert('Fehler: '+e.message);}
}

function simCellHtml(g){
  if(g.sim_win_ratio==null)return '<span class="neu">–</span>';
  const pct=g.sim_win_ratio*100;
  const cls=pct>=66?'pos':pct>=33?'neu':'neg';
  const mush=g.sim_mushrooms?` <span title="Pilze im Katapult">${MUSH_ICON}${g.sim_mushrooms}</span>`:'';
  return `<span class="${cls}">${pct.toFixed(1)} %</span>${mush}`;
}

function renderTable(){
  const tb=$('rBody'); tb.innerHTML='';

  document.querySelectorAll('#rTable th .arrow').forEach(a=>a.textContent='');
  const arrow=$('sort_'+sortKey);
  if(arrow)arrow.textContent=sortDir==='asc'?' ▲':' ▼';

  const sorted=[...results].sort((a,b)=>{
    let va=a[sortKey],vb=b[sortKey];
    if(sortKey==='sim_win_ratio'){va=va??-1;vb=vb??-1;}
    if(typeof va==='string'){va=va.toLowerCase();vb=vb.toLowerCase();}
    if(va<vb)return sortDir==='asc'?-1:1;
    if(va>vb)return sortDir==='asc'?1:-1;
    return 0;
  });

  const ownAvg=(ownGuild && typeof ownGuild.avg_level==='number')?ownGuild.avg_level:(ownGuild.member_count>0?ownGuild.members.reduce((s,m)=>s+m.level,0)/ownGuild.member_count:0);
  const ownMax=Math.max(...ownGuild.members.map(m=>m.level));

  sorted.forEach(g=>{
    const tr=document.createElement('tr');
    const avgDiff=(g.avg_level??((g.member_count>0)?(g.total_level/g.member_count):0)) - ownAvg;
    const avgCls=avgDiff<0?'pos':avgDiff>0?'neg':'neu';
    const memberDiff=g.member_count-ownGuild.member_count;
    const memberCls=memberDiff<0?'pos':memberDiff>0?'neg':'neu';
    const maxCls=g.max_level<ownMax?'pos':g.max_level>ownMax?'neg':'neu';
    const atk=g.is_attacked?'<span class="badge b-red">Angegriffen</span>':'<span class="badge b-grn">Frei</span>';
    const strictBadge=g.strict_evaluated?(g.strict_beatable?'<span class="badge b-grn">✓ Strikt</span>':'<span class="badge b-red">✗ Strikt</span>'):'';

    tr.innerHTML=`
      <td><input type="checkbox" class="gsel" data-name="${esc(g.name)}" ${selected.rBody.has(g.name)?'checked':''} onchange="toggleGuildSelection(this)"></td>
      <td class="name">${esc(g.name)}</td>
      <td class="num ${memberCls}">${g.member_count}</td>
      <td class="num ${maxCls}">${g.max_level}</td>
      <td class="num">${g.min_level}</td>
      <td class="num ${avgCls}">${fmt1(g.avg_level)}</td>
      <td class="num">#${g.rank}</td>
      <td class="num">${g.honor.toLocaleString('de')}</td>
      <td class="num">${simCellHtml(g)}</td>
      <td>${atk}${strictBadge}</td>
      <td>${rowActionsHtml(g.name)} <button class="btn sec small" type="button" data-g="${esc(g.name)}" onclick="showDetail(this.dataset.g)">Details</button></td>`;
    tb.appendChild(tr);
  });

  applyCols('rTable');
  // Erst jetzt, mit den tatsächlich gerenderten (ggf. vorbelegten) Checkboxen,
  // Zähler + Header-Checkbox-Zustand berechnen.
  updateSelInfo();
}

function sortBy(key){
  if(sortKey===key)sortDir=sortDir==='asc'?'desc':'asc';
  else{sortKey=key;sortDir='asc';}
  renderTable();
}

// ── VOLLSTÄNDIGE EHRENHALLE ──────────────────────────────────
function sortHofBy(key){
  if(hofSortKey===key)hofSortDir=hofSortDir==='asc'?'desc':'asc';
  else{hofSortKey=key;hofSortDir='asc';}
  renderHofTable();
}

function renderHofTable(){
  const maxM=$('fHofMembers').value?parseInt($('fHofMembers').value):null;
  const onlyNotAttackable=$('fHofOnlyNotAttackable').checked;

  const visible=hofResults.filter(g=>{
    if(maxM!=null && g.member_count>maxM) return false;
    if(onlyNotAttackable && g.attackable) return false;
    return true;
  });

  $('hofCount').textContent=`${visible.length} / ${hofResults.length} Gilden`;

  document.querySelectorAll('#hofTable th .arrow').forEach(a=>a.textContent='');
  const arrow=$('hsort_'+hofSortKey);
  if(arrow)arrow.textContent=hofSortDir==='asc'?' ▲':' ▼';

  const sorted=[...visible].sort((a,b)=>{
    let va=a[hofSortKey],vb=b[hofSortKey];
    if(hofSortKey==='sim_win_ratio'){va=va??-1;vb=vb??-1;}
    if(typeof va==='boolean'){va=va?1:0;vb=vb?1:0;}
    if(typeof va==='string'){va=va.toLowerCase();vb=vb.toLowerCase();}
    if(va<vb)return hofSortDir==='asc'?-1:1;
    if(va>vb)return hofSortDir==='asc'?1:-1;
    return 0;
  });

  const tb=$('hofBody'); tb.innerHTML='';
  sorted.forEach(g=>{
    const tr=document.createElement('tr');
    const atkBadge=g.attackable?'<span class="badge b-grn">✓ Ja</span>':'<span class="badge b-red">✗ Nein</span>';
    tr.innerHTML=`
      <td><input type="checkbox" class="gsel" data-name="${esc(g.name)}" ${selected.hofBody.has(g.name)?'checked':''} onchange="toggleGuildSelection(this)"></td>
      <td class="num">#${g.rank}</td>
      <td class="name">${esc(g.name)}</td>
      <td class="num">${g.member_count}</td>
      <td class="num">${g.honor.toLocaleString('de')}</td>
      <td>${atkBadge}</td>
      <td class="num">${simCellHtml(g)}</td>
      <td>${rowActionsHtml(g.name)} <button class="btn sec small" type="button" data-g="${esc(g.name)}" onclick="showDetail(this.dataset.g)">Details</button></td>`;
    tb.appendChild(tr);
  });

  applyCols('hofTable');
  updateSelInfo();
}

// ── CHECKBOX SELECTION ─────────────────────────────────────
// Jede Tabelle (Ergebnisse `rBody`, Vollständige Ehrenhalle `hofBody`) hat ihre
// eigene Auswahl. Eine Gilde, die in beiden Tabellen steht, wird so nur dort
// simuliert, wo sie angehakt wurde. `selected` übersteht Neuaufbauten der
// Tabelle (z.B. nach einer Simulation); die Checkboxen sind nur die Darstellung.
function scopeOf(cb){return cb.closest('tbody').id;}

function toggleGuildSelection(cb){
  const set=selected[scopeOf(cb)];
  if(cb.checked) set.add(cb.dataset.name); else set.delete(cb.dataset.name);
  updateSelInfo();
}

function setScope(scopeId,on){
  $(scopeId).querySelectorAll('.gsel').forEach(c=>{
    c.checked=on;
    if(on) selected[scopeId].add(c.dataset.name); else selected[scopeId].delete(c.dataset.name);
  });
  updateSelInfo();
}
function toggleAll(cb,scopeId){setScope(scopeId,cb.checked);}
function selectAllVisible(scopeId){setScope(scopeId,true);}
function deselectAll(scopeId){setScope(scopeId,false);}

function updateSelInfo(){
  [['rBody','selInfo','selAll'],['hofBody','selInfoHof','selAllHof']].forEach(([scope,infoId,allId])=>{
    const total=$(scope).querySelectorAll('.gsel').length;
    const checked=$(scope).querySelectorAll('.gsel:checked').length;
    $(infoId).textContent=`${checked} von ${total} Gilden ausgewählt`;
    syncSelectAllCheckbox(scope,allId);
  });
}

function syncSelectAllCheckbox(scopeId, checkboxId){
  const scope=$(scopeId), cb=$(checkboxId);
  if(!scope || !cb) return;
  const total=scope.querySelectorAll('.gsel').length;
  const checked=scope.querySelectorAll('.gsel:checked').length;
  if(total>0){
    cb.indeterminate = checked>0 && checked<total;
    if(checked===total) cb.checked=true;
    if(checked===0) cb.checked=false;
  } else {
    cb.indeterminate=false;
    cb.checked=false;
  }
}

// ── TXT EXPORT ─────────────────────────────────────────────
function exportTxt(){
  const checked=[...$('rBody').querySelectorAll('.gsel:checked')].map(c=>c.dataset.name);
  if(!checked.length){
    alert('Keine Gilden ausgewählt.\nBitte zuerst die gewünschten Gilden per Checkbox markieren.');
    return;
  }
  const content=checked.join('/');
  const blob=new Blob([content],{type:'text/plain;charset=utf-8'});
  const a=document.createElement('a');
  a.href=URL.createObjectURL(blob);
  a.download='gilden_angriff.txt';
  document.body.appendChild(a);
  a.click();
  document.body.removeChild(a);
  URL.revokeObjectURL(a.href);
}

// ── KAMPF-SIMULATION ───────────────────────────────────────
async function doRefreshOwnFighters(){
  $('refreshFightersBtn').disabled=true;
  $('refreshFightersMsg').textContent='Lädt eigene Kampfwerte (Ausrüstung/Attribute) neu...';
  try{
    const r=await api('refresh-own-fighters',{});
    $('refreshFightersMsg').textContent=r.success?('✓ '+r.data):('Fehler: '+r.error);
  }catch(e){$('refreshFightersMsg').textContent='Fehler: '+e.message;}
  finally{$('refreshFightersBtn').disabled=false;}
}

async function doSimulate(mushroomSelectId){
  // Nur die Gilden der Tabelle, von der aus gestartet wurde
  const scope=mushroomSelectId==='simMushroomsHof'?'hofBody':'rBody';
  const checked=[...$(scope).querySelectorAll('.gsel:checked')].map(c=>c.dataset.name);
  if(!checked.length){
    alert('Keine Gilden ausgewählt.\nBitte zuerst die gewünschten Gilden per Checkbox markieren.');
    return;
  }
  startSim(checked,$(mushroomSelectId||'simMushrooms').value||'0');
}

function doSimulateRow(btn){
  startSim([btn.dataset.g],btn.closest('.rowact').querySelector('.rowMush').value);
}

async function startSim(names,mush){
  lastMushrooms=String(mush);
  const mushrooms=parseInt(mush||'0');

  // Es kann immer nur eine Simulation gleichzeitig laufen, deshalb sind
  // Buttons und Fortschritt für beide Seiten (Ergebnisse + Ehrenhalle) gemeinsam.
  simUi(el=>{el.disabled=true;},'simBtn');
  simUi(el=>{el.disabled=false;},'simAbortBtn');
  simUi(el=>el.classList.remove('hidden'),'simProgress');
  simUi(el=>el.classList.remove('neg'),'simMsg');

  try{
    const p=participation();
    const r=await api('simulate',{guild_names:names,mushrooms,include_members:p.include,exclude_members:p.exclude});
    if(!r.success){simError(r.error); return;}
    pollSimProgress();
  }catch(e){simError('Fehler: '+e.message);}
}

function simError(msg){
  simUi(el=>{el.textContent=msg; el.classList.add('neg');},'simMsg');
  simUi(el=>{el.disabled=false;},'simBtn');
  simUi(el=>{el.disabled=true;},'simAbortBtn');
}

async function doAbortSim(){
  simUi(el=>{el.disabled=true;},'simAbortBtn');
  try{
    const r=await api('simulate/abort',{});
    simUi(el=>{el.textContent=r.success?(r.data||'Abbruch angefordert...'):r.error; el.classList.toggle('neg',!r.success);},'simMsg');
  }catch(e){simError('Fehler: '+e.message);}
}

async function pollSimProgress(){
  const fill=(w,txt)=>{simUi(el=>{el.style.width=w;},'simFill'); simUi(el=>{el.textContent=txt;},'simText');};
  const msg=m=>simUi(el=>{el.textContent=m;},'simMsg');
  const idle=()=>{simUi(el=>{el.disabled=false;},'simBtn'); simUi(el=>{el.disabled=true;},'simAbortBtn');};
  try{
    const r=await api('simulate/progress');
    if(!r.success)return;
    const p=r.data;
    const pct=p.total>0?Math.round(p.current/p.total*100):0;

    if(p.phase==='own-roster'){
      fill('5%','Eigene Kampfwerte...');
    }else if(p.phase==='guilds'){
      fill(pct+'%',`${p.current} / ${p.total} Gilden`);
    }else if(p.phase==='done' || p.phase==='aborted'){
      fill('100%',p.phase==='done'?'Fertig!':'Abgebrochen');
      idle();
      msg(p.message);
      applyFilter();
      return;
    }

    msg(p.message);
    if(p.running) setTimeout(pollSimProgress, 800);
    else {
      idle();
      if(p.phase==='error') simUi(el=>el.classList.add('neg'),'simMsg');
    }
  }catch(e){setTimeout(pollSimProgress,2000);}
}

// ── DETAIL MODAL ───────────────────────────────────────────
async function showDetail(name){
  $('mTitle').textContent='Lade...';
  $('mBody').innerHTML='<div class="empty" style="text-align:center">Lädt...</div>';
  $('modal').classList.remove('hidden');

  const cached=results.find(g=>g.name===name);
  if(cached){renderDetail(cached);return;}

  try{
    const r=await api('guild-details',{name});
    if(!r.success){$('mTitle').textContent='Fehler';$('mBody').textContent=r.error;return;}
    renderDetail(r.data);
  }catch(e){$('mTitle').textContent='Fehler';$('mBody').textContent=e.message;}
}

function renderDetail(g){
  $('mTitle').textContent=g.name;
  const maxLvl=g.max_level||Math.max(...g.members.map(m=>m.level));

  const ownAvg=(ownGuild && typeof ownGuild.avg_level==='number')?ownGuild.avg_level:(ownGuild.member_count>0?ownGuild.members.reduce((s,m)=>s+m.level,0)/ownGuild.member_count:0);
  const ownMax=Math.max(...ownGuild.members.map(m=>m.level));
  const gAvg=(g.member_count>0)?(g.total_level/g.member_count):0;

  let cmp='';
  if(ownGuild){
    const avgDiff=gAvg-ownAvg;
    const memberDiff=g.member_count-ownGuild.member_count;
    const maxDiff=maxLvl-ownMax;
    cmp=`<div class="infobox">
      <strong>Vergleich mit ${esc(ownGuild.name)}:</strong><br>
      Mitglieder: <span class="${memberDiff<=0?'pos':'neg'}">${memberDiff>0?'+':''}${memberDiff}</span> |
      Ø Level: <span class="${avgDiff<=0?'pos':'neg'}">${avgDiff>0?'+':''}${fmt1(avgDiff)}</span> |
      Max Level: <span class="${maxDiff<=0?'pos':'neg'}">${maxDiff>0?'+':''}${maxDiff}</span>
    </div>`;
  }

  let strictHtml='';
  if(g.strict_evaluated){
    const mode=g.strict_topn?' (Top-N)':'';
    const badge=g.strict_beatable?'<span class="badge b-grn">✓ Strikt besiegbar</span>':'<span class="badge b-red">✗ Strikt NICHT</span>';
    const reason=(!g.strict_beatable && g.strict_fail_reason)?`<div class="hint" style="margin-top:0.35rem">${esc(g.strict_fail_reason)}</div>`:'';
    strictHtml=`<div class="infobox">
      <strong>Strikt-Modus${mode}:</strong> ${badge}<br>
      Eigene kampfbereit: <span class="b-gld badge">${g.strict_own_active_members}</span> | Gegner: <span class="b-gld badge">${g.member_count}</span>${g.strict_topn ? (' | Top-N: <span class="b-gld badge">'+g.strict_topn_n+'</span>') : ''}
      ${reason}
    </div>`;
  }

  let simHtml='';
  if(g.sim_win_ratio!=null){
    const pct=g.sim_win_ratio*100;
    const cls=pct>=66?'pos':pct>=33?'neu':'neg';
    const badge=`<span class="${cls}" style="font-weight:700">${pct.toFixed(1)} %</span>`;
    simHtml=`<div class="infobox">
      <strong>Kampfsimulation:</strong> Sieg-Wahrscheinlichkeit ${badge}<br>
      ${g.sim_mushrooms?`Pilzkatapult: <span class="b-gld badge">${MUSH_ICON}${g.sim_mushrooms}</span> | `:''}${g.sim_iterations} simulierte Kämpfe | Stand: ${esc(g.sim_evaluated_at||'')}
    </div>`;
  }

  const rows=g.members.sort((a,b)=>b.level-a.level).map(m=>
    `<tr><td>${esc(m.name)}</td><td>${m.level} ${levelBar(m.level,maxLvl)}</td></tr>`
  ).join('');

  $('mBody').innerHTML=`
    <div class="stats">
      <div class="stat"><b>${g.member_count}</b><span>Mitglieder</span></div>
      <div class="stat"><b>${g.max_level}</b><span>Max Level</span></div>
      <div class="stat"><b>${g.min_level}</b><span>Min Level</span></div>
      <div class="stat"><b>${fmt1((g.member_count>0)?(g.total_level/g.member_count):0)}</b><span>Ø Level</span></div>
      <div class="stat"><b>#${g.rank}</b><span>Rang</span></div>
      <div class="stat"><b>${g.honor.toLocaleString('de')}</b><span>Ehre</span></div>
    </div>
    ${cmp}
    ${strictHtml}
    ${simHtml}
    <div class="tablewrap"><table><thead><tr><th>Name</th><th>Level</th></tr></thead><tbody>${rows}</tbody></table></div>`;
}

// ── LOGOUT ─────────────────────────────────────────────────
async function doLogout(){
  await api('auth/logout',{});
  location.reload();
}

function esc(s){const d=document.createElement('div');d.textContent=s;return d.innerHTML.replace(/"/g,'&quot;');}

// ── VERSION ────────────────────────────────────────────────
(async function(){
  try{
    const r=await api('version');
    if(r.success && r.data){
      document.querySelectorAll('.appVersion').forEach(el=>{el.textContent=`v${r.data}`;});
      document.title=`SF Gilden-Scanner v${r.data}`;
    }
  }catch(e){/* Version ist rein kosmetisch, kein Fehler nötig */}
})();

(function(){
  const sm=$('strictMode');
  const st=$('strictTopN');
  if(sm && st){
    const sync=()=>{st.disabled=!sm.checked; st.closest('label').style.opacity=sm.checked?'1':'0.5';};
    sm.addEventListener('change', sync);
    sync();
  }
})();

// ── EINSTELLUNGEN & UPDATE-HINWEIS ─────────────────────────
async function loadSettings(){
  try{
    const r=await api('settings');
    if(r.success)$('setUpdate').value=r.data.update_check;
  }catch(e){/* Einstellungen sind optional, Standard bleibt sichtbar */}
}
$('setUpdate').addEventListener('change',async()=>{
  try{
    const r=await api('settings',{update_check:$('setUpdate').value});
    $('setUpdateMsg').textContent=r.success?'Gespeichert.':'Fehler: '+r.error;
    checkUpdate();
  }catch(e){$('setUpdateMsg').textContent='Fehler: '+e.message;}
});
async function checkUpdate(){
  try{
    const r=await api('update-check');
    if(!r.success)return;
    const u=r.data, el=$('updHint');
    if(u.enabled&&u.newer){
      el.textContent=`Update ${u.latest} verfügbar`; el.href=u.url; el.classList.remove('hidden');
    }else el.classList.add('hidden');
    $('updStatus').textContent=!u.enabled?'Die Prüfung ist aus.'
      :u.latest?`Installiert: ${u.current||'unbekannt'} · Neueste: ${u.latest}`
      :'Die neueste Version konnte noch nicht ermittelt werden.';
  }catch(e){/* ohne Netz bleibt der Hinweis einfach aus */}
}

// ── ANMELDUNG AM SCANNER ───────────────────────────────────
let authUser=null, authMode='login', appStarted=false;

function showAuth(mode){
  authMode=mode;
  authUser=null;
  $('authPass2Field').classList.toggle('hidden',mode!=='setup');
  $('authIntro').textContent=mode==='setup'
    ?'Willkommen! Lege den ersten Benutzer an. Er wird Admin und kann danach weitere Benutzer anlegen.'
    :'';
  $('authBtn').textContent=mode==='setup'?'Admin anlegen':'Anmelden';
  $('authPass').autocomplete=mode==='setup'?'new-password':'current-password';
  $('authErr').textContent='';
  view='auth'; route();
}

async function doAuth(){
  const name=$('authName').value.trim(), pass=$('authPass').value;
  if(!name||!pass)return;
  if(authMode==='setup'&&pass!==$('authPass2').value){$('authErr').textContent='Die Passwörter stimmen nicht überein';return;}
  $('authBtn').disabled=true; $('authErr').textContent='';
  try{
    const r=await api(authMode==='setup'?'auth/setup':'auth/login',{name,password:pass});
    if(!r.success){$('authErr').textContent=r.error;return;}
    $('authPass').value=''; $('authPass2').value='';
    enterApp(r.data);
  }catch(e){$('authErr').textContent='Verbindungsfehler';}
  finally{$('authBtn').disabled=false;}
}
$('authPass').addEventListener('keydown',e=>{if(e.key==='Enter'&&authMode==='login')doAuth();});
$('authPass2').addEventListener('keydown',e=>{if(e.key==='Enter')doAuth();});

function enterApp(user){
  authUser=user;
  updateWho();
  $('navUsers').classList.toggle('hidden',!user.is_admin);
  $('setUpdate').disabled=!user.is_admin;
  loadSettings(); checkUpdate();
  if(!appStarted){appStarted=true; setInterval(checkUpdate,30*60*1000);}
  restore();
}

async function boot(){
  try{
    const r=await api('auth/state');
    if(!r.success){showAuth('login');return;}
    if(r.data.setup_needed){showAuth('setup');return;}
    if(!r.data.user){showAuth('login');return;}
    enterApp(r.data.user);
  }catch(e){showAuth('login');}
}

// ── GESPEICHERTE KONTEN ────────────────────────────────────
let accountsInfo=null;

function guildText(c){
  if(c.guild===null||c.guild===undefined)return 'Gilde unbekannt';
  return c.guild?c.guild:'keine Gilde';
}

function isActiveChar(c){
  return !!activeChar&&activeChar.name===c.name&&activeChar.server===c.server;
}

function renderStored(list){
  $('storedCard').classList.toggle('hidden',!list.length);
  $('noAccountsHint').classList.toggle('hidden',list.length>0);
  const l=$('storedList'); l.innerHTML='';
  list.forEach(c=>{
    const b=document.createElement('button');
    b.type='button';
    b.className='charcard'+(isActiveChar(c)?' on':'');
    b.innerHTML=`<b>${esc(c.name)}</b><small>${esc(c.server)} · ${esc(guildText(c))} · Konto ${esc(c.account)}</small>`;
    b.onclick=()=>selectStored(c);
    l.appendChild(b);
  });
}

async function selectStored(c){
  const btns=$('storedList').querySelectorAll('button'); btns.forEach(b=>b.disabled=true);
  try{
    const r=await api('accounts/select',{account_id:c.account_id,name:c.name,server:c.server});
    if(!r.success){alert(r.error);return;}
    // Stand eines vorher gewählten Charakters nicht mitschleppen.
    resetResults();
    activeChar={name:c.name,server:c.server};
    showGuild(r.data);
    goPage('overview');
    const st=await api('status');
    applyScanStatus(st.data);
  }catch(e){alert('Fehler: '+e.message);}
  finally{btns.forEach(b=>b.disabled=false);}
}

async function loadAccounts(){
  let r;
  try{r=await api('accounts');}catch(e){r={success:false,error:e.message};}
  if(!r.success){
    $('accountsList').innerHTML=`<div class="card err">Konten konnten nicht gelesen werden: ${esc(r.error||'')}</div>`;
    return;
  }
  accountsInfo=r.data;
  $('keyWarn').classList.toggle('hidden',r.data.key_set);
  $('accAddCard').classList.toggle('hidden',!r.data.key_set);
  renderAccounts();
}

const openAccs=new Set();
let accsSeen=false;

function renderAccounts(){
  const box=$('accountsList'); box.innerHTML='';
  if(!accountsInfo.accounts.length){
    box.innerHTML='<div class="card empty">Noch keine Konten hinterlegt.</div>';
    return;
  }
  // Beim ersten Anzeigen nur dann aufgeklappt, wenn es genau ein Konto gibt.
  if(!accsSeen){
    accsSeen=true;
    if(accountsInfo.accounts.length===1)openAccs.add(accountsInfo.accounts[0].id);
  }
  accountsInfo.accounts.forEach(a=>{
    const card=document.createElement('div');
    card.className='card acc';
    const chars=a.characters.map(c=>`<label class="char${c.active?' on':''}"><span><b>${esc(c.name)}</b><small>${esc(c.server)} · ${esc(guildText(c))}</small></span><span class="check"><input type="checkbox" data-name="${esc(c.name)}" data-server="${esc(c.server)}" ${c.active?'checked':''}></span></label>`).join('');
    const open=openAccs.has(a.id);
    const activeCount=a.characters.filter(c=>c.active).length;
    card.innerHTML=`<div class="acc-head"><h3 style="margin:0"><button class="acc-toggle" type="button" aria-expanded="${open}"><span class="chev" aria-hidden="true">${open?'▾':'▸'}</span>${esc(a.sso_user)}<span class="hint">${activeCount} von ${a.characters.length} aktiv</span></button></h3><span class="hint" data-msg></span><span class="grow" style="flex:1"></span>
        <button class="btn sec small" type="button" data-act="refresh">Gilden aktualisieren</button>
        <button class="btn sec small" type="button" data-act="password">Passwort ändern</button>
        <button class="btn red small" type="button" data-act="delete">Entfernen</button></div>
      <div class="acc-body${open?'':' hidden'}">
      <p class="hint">Haken = Charakter ist für Scan und Simulation nutzbar und erscheint unter "Charakter wechseln".</p>
      <div class="chars">${chars}</div>
      <div class="row" style="margin-top:0.7rem"><button class="btn" type="button" data-act="save">Auswahl speichern</button></div></div>`;
    card.querySelector('.acc-toggle').addEventListener('click',e=>{
      const t=e.currentTarget, body=card.querySelector('.acc-body'), show=body.classList.contains('hidden');
      body.classList.toggle('hidden',!show);
      if(show)openAccs.add(a.id); else openAccs.delete(a.id);
      t.setAttribute('aria-expanded',show);
      t.querySelector('.chev').textContent=show?'▾':'▸';
    });
    card.querySelectorAll('.char input').forEach(i=>i.addEventListener('change',()=>i.closest('.char').classList.toggle('on',i.checked)));
    card.querySelectorAll('[data-act]').forEach(b=>b.addEventListener('click',()=>accountAction(a,b.dataset.act,card)));
    box.appendChild(card);
  });
}

function replaceAccount(view){
  accountsInfo.accounts=accountsInfo.accounts.map(a=>a.id===view.id?view:a);
  renderAccounts();
}

async function accountAction(a,act,card){
  const msg=card.querySelector('[data-msg]');
  const busy=on=>card.querySelectorAll('button').forEach(b=>{b.disabled=on;});
  try{
    if(act==='save'||act==='refresh'){
      const body=act==='save'
        ?{id:a.id,active:[...card.querySelectorAll('.char input:checked')].map(i=>({name:i.dataset.name,server:i.dataset.server}))}
        :{id:a.id};
      busy(true); msg.textContent='Bitte warten, die Gilden werden beim Spiel abgefragt...';
      const r=await api(act==='save'?'accounts/activate':'accounts/refresh',body);
      if(!r.success){busy(false); msg.textContent='Fehler: '+r.error; return;}
      replaceAccount(r.data);
    }else if(act==='password'){
      openAccPwModal(a);
    }else if(act==='delete'){
      if(!confirm(`Konto "${a.sso_user}" samt gespeichertem Passwort entfernen?`))return;
      const r=await api('accounts/delete',{id:a.id});
      if(!r.success){alert(r.error);return;}
      openAccs.delete(a.id);
      const st=await api('status');
      if(st.success&&!st.data.guild_loaded&&ownGuild){location.reload();return;}
      await loadAccounts();
    }
  }catch(e){busy(false); msg.textContent='Fehler: '+e.message;}
}

function openAccPwModal(a){
  $('mTitle').textContent=`Passwort ändern: ${a.sso_user}`;
  $('mBody').innerHTML=`<div class="form" style="grid-template-columns:1fr"><div class="field"><label for="accNewPw">Neues S&F-Passwort</label><input type="password" id="accNewPw" autocomplete="new-password"></div></div>
    <button class="btn" type="button" id="accNewPwBtn">Prüfen und speichern</button> <span class="hint" id="accNewPwMsg">Das Passwort wird beim Spiel geprüft.</span>`;
  $('modal').classList.remove('hidden');
  $('accNewPwBtn').onclick=async()=>{
    $('accNewPwBtn').disabled=true; $('accNewPwMsg').textContent='Prüfe...';
    const r=await api('accounts/password',{id:a.id,password:$('accNewPw').value});
    $('accNewPwMsg').textContent=r.success?'Gespeichert.':'Fehler: '+r.error;
    $('accNewPwBtn').disabled=false;
    if(r.success)setTimeout(()=>$('modal').classList.add('hidden'),900);
  };
}

async function addAccount(){
  const sso_user=$('accUser').value.trim(), password=$('accPass').value;
  if(!sso_user||!password)return;
  $('accAddBtn').disabled=true; $('accAddMsg').textContent='Prüfe das Konto beim Spiel...';
  try{
    const r=await api('accounts/add',{sso_user,password});
    if(!r.success){$('accAddMsg').textContent='Fehler: '+r.error;return;}
    $('accUser').value=''; $('accPass').value='';
    openAccs.add(r.data.id);
    $('accAddMsg').textContent=`Konto "${r.data.sso_user}" gespeichert. Aktiviere jetzt die Charaktere, mit denen du arbeiten willst.`;
    await loadAccounts();
  }catch(e){$('accAddMsg').textContent='Fehler: '+e.message;}
  finally{$('accAddBtn').disabled=false;}
}

// ── BENUTZERVERWALTUNG (Admin) ─────────────────────────────
async function loadUsers(){
  const r=await api('admin/users');
  if(!r.success)return;
  const tb=$('usersTable').querySelector('tbody'); tb.innerHTML='';
  r.data.forEach(u=>{
    const tr=document.createElement('tr');
    const self=authUser&&u.id===authUser.id;
    tr.innerHTML=`<td class="name">${esc(u.name)}${self?' <span class="hint">(du)</span>':''}</td>
      <td>${u.is_admin?'<span class="badge b-gld">Admin</span>':'Benutzer'}</td>
      <td>${esc(u.created)}</td>
      <td><span class="rowbtns"><button class="btn sec small" type="button" data-id="${esc(u.id)}" data-name="${esc(u.name)}" onclick="openPwModal(this.dataset.id,this.dataset.name)">Passwort setzen</button>${self?'':`<button class="btn red small" type="button" data-id="${esc(u.id)}" data-name="${esc(u.name)}" onclick="deleteUser(this.dataset.id,this.dataset.name)">Löschen</button>`}</span></td>`;
    tb.appendChild(tr);
  });
}

async function createUser(){
  const name=$('newUserName').value.trim(), password=$('newUserPass').value;
  const r=await api('admin/users',{name,password,is_admin:$('newUserAdmin').checked});
  $('newUserMsg').textContent=r.success?`Benutzer "${r.data.name}" angelegt.`:'Fehler: '+r.error;
  if(r.success){$('newUserName').value=''; $('newUserPass').value=''; $('newUserAdmin').checked=false; loadUsers();}
}

async function deleteUser(id,name){
  if(!confirm(`Benutzer "${name}" samt allen Scan-Daten löschen?`))return;
  const r=await api('admin/users/delete',{id});
  if(!r.success)alert(r.error);
  loadUsers();
}

function openPwModal(id,name){
  $('mTitle').textContent=`Passwort setzen: ${name}`;
  $('mBody').innerHTML=`<div class="form" style="grid-template-columns:1fr"><div class="field"><label for="admPw">Neues Passwort</label><input type="password" id="admPw" autocomplete="new-password"></div></div>
    <button class="btn" type="button" id="admPwBtn">Setzen</button> <span class="hint" id="admPwMsg">Mindestens 10 Zeichen. Alle Sitzungen des Benutzers enden.</span>`;
  $('modal').classList.remove('hidden');
  $('admPwBtn').onclick=async()=>{
    const r=await api('admin/users/password',{id,password:$('admPw').value});
    $('admPwMsg').textContent=r.success?'Passwort gesetzt.':'Fehler: '+r.error;
    if(r.success)setTimeout(()=>$('modal').classList.add('hidden'),900);
  };
}

async function changePassword(){
  if($('pwNew').value!==$('pwNew2').value){$('pwMsg').textContent='Die neuen Passwörter stimmen nicht überein';return;}
  const r=await api('auth/password',{old:$('pwOld').value,new:$('pwNew').value});
  $('pwMsg').textContent=r.success?'Passwort geändert.':'Fehler: '+r.error;
  if(r.success){$('pwOld').value=''; $('pwNew').value=''; $('pwNew2').value='';}
}

initColMenus();

// Nach einem Seiten-Refresh hält der Server die Sitzung noch: Zustand
// wiederherstellen statt wieder den Login zu zeigen.
async function restore(){
  let st;
  try{
    st=await api('status');
    if(!st.success||!st.data.logged_in){view='app'; goPage('character'); return;}
    if(!(st.data.guild_loaded&&st.data.own_guild)){view='app'; goPage('character'); return;}
    activeChar=st.data.character_name?{name:st.data.character_name,server:st.data.character_server||''}:null;
    showGuild(st.data.own_guild);
    view='app'; route();
    applyScanStatus(st.data);
  }catch(e){console.error('Wiederherstellung fehlgeschlagen',e); view='app'; goPage('character'); return;}

  try{
    if(st.data.has_scan_data)await applyFilter();
    const sp=await api('progress');
    if(sp.data?.running){
      $('progressArea').classList.remove('hidden');
      $('scanBtn').disabled=true; $('abortBtn').disabled=false;
      pollProgress();
    }
    const mp=await api('simulate/progress');
    if(mp.data?.running){
      simUi(el=>el.classList.remove('hidden'),'simProgress');
      simUi(el=>{el.disabled=true;},'simBtn'); simUi(el=>{el.disabled=false;},'simAbortBtn');
      pollSimProgress();
    }
  }catch(e){console.error('Laufende Aufgaben konnten nicht wiederhergestellt werden',e);}
}

route();
boot();
