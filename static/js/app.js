const $=id=>document.getElementById(id);
const t=key=>(window.I18N.de&&window.I18N.de[key])||key;

let results=[], ownGuild=null;
let sortKey='rank', sortDir='asc';
let hofResults=[];
let hofSortKey='rank', hofSortDir='asc';
let hasScanData=false, scanDate='';
// Namen der per Checkbox ausgewählten Gilden — bleibt über renderTable()-
// Neuaufbauten hinweg bestehen (z.B. nach applyFilter() nach Simulationsende),
// damit die Häkchen nicht bei jedem Neurendern verschwinden.
let selectedGuilds=new Set();

const PAGES=['overview','scan','results','hof','help'];
let view='login';

document.querySelectorAll('[data-i18n]').forEach(el=>{el.textContent=t(el.dataset.i18n);});

// ── ANSICHTEN / NAVIGATION ─────────────────────────────────
function route(){
  $('loginView').classList.toggle('hidden',view!=='login');
  $('charsView').classList.toggle('hidden',view!=='chars');
  $('appView').classList.toggle('hidden',view!=='app');
  if(view!=='app')return;
  let page=(location.hash||'#overview').slice(1);
  if(!PAGES.includes(page))page='overview';
  document.querySelectorAll('.page').forEach(p=>p.classList.toggle('show',p.id==='p-'+page));
  document.querySelectorAll('.nbtn[data-page]').forEach(b=>b.classList.toggle('active',b.dataset.page===page));
  $('side').classList.remove('open');
  window.scrollTo(0,0);
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
function showChars(chars){
  view='chars'; route();
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
  results=[]; hofResults=[]; selectedGuilds=new Set();
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
  // Stand eines evtl. vorher gewählten Charakters (bei Charakterwechsel)
  // nicht mit rüberschleppen.
  resetResults();
  try{
    const r=await api('select-character',{index:idx});
    if(!r.success){alert(r.error);return;}
    showGuild(r.data);
    view='app'; location.hash='#overview'; route();
    const st=await api('status');
    applyScanStatus(st.data);
  }catch(e){alert('Fehler: '+e.message);}
  finally{$('charList').querySelectorAll('button').forEach(b=>b.disabled=false);}
}

// Wechselt zu einem anderen Charakter auf demselben SSO-Account, ohne
// sich neu einzuloggen (alle Charaktere sind schon aus dem Login bekannt).
async function doSwitchCharacter(){
  try{
    const r=await api('characters');
    if(!r.success){alert(r.error);return;}
    $('modal').classList.add('hidden');
    showChars(r.data);
  }catch(e){alert('Fehler: '+e.message);}
}

// ── OWN GUILD ──────────────────────────────────────────────
function showGuild(g){
  ownGuild=g;
  $('guildName').textContent=g.name;
  $('ovSub').textContent=`Rang #${g.rank}`;
  $('who').textContent=g.name;

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

  const tb=$('ownMembers').querySelector('tbody'); tb.innerHTML='';
  [...g.members].sort((a,b)=>b.level-a.level).forEach(m=>{
    const tr=document.createElement('tr');
    const off=(m.offline_days!=null && m.offline_days>=1)?` <span class="badge b-red" title="offline seit ${m.offline_days} Tagen">off ${m.offline_days}d</span>`:'';
    const ready=(m.is_active_24h!==false)&&(m.attack_joined!==false);
    tr.className=ready?'':'dim';
    tr.innerHTML=`<td>${esc(m.name)}${off}</td><td>${m.level} ${levelBar(m.level,ownMax)}</td>`;
    tb.appendChild(tr);
  });
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
  const mush=g.sim_mushrooms?` <span title="Pilze im Katapult">· ${g.sim_mushrooms}</span>`:'';
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
      <td><input type="checkbox" class="gsel" data-name="${esc(g.name)}" ${selectedGuilds.has(g.name)?'checked':''} onchange="toggleGuildSelection(this)"></td>
      <td class="name">${esc(g.name)}</td>
      <td class="num ${memberCls}">${g.member_count}</td>
      <td class="num ${maxCls}">${g.max_level}</td>
      <td class="num">${g.min_level}</td>
      <td class="num ${avgCls}">${fmt1(g.avg_level)}</td>
      <td class="num">#${g.rank}</td>
      <td class="num">${g.honor.toLocaleString('de')}</td>
      <td class="num">${simCellHtml(g)}</td>
      <td>${atk}${strictBadge}</td>
      <td><button class="btn sec small" type="button" data-g="${esc(g.name)}" onclick="showDetail(this.dataset.g)">Details</button></td>`;
    tb.appendChild(tr);
  });

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
      <td><input type="checkbox" class="gsel" data-name="${esc(g.name)}" ${selectedGuilds.has(g.name)?'checked':''} onchange="toggleGuildSelection(this)"></td>
      <td class="num">#${g.rank}</td>
      <td class="name">${esc(g.name)}</td>
      <td class="num">${g.member_count}</td>
      <td class="num">${g.honor.toLocaleString('de')}</td>
      <td>${atkBadge}</td>
      <td class="num">${simCellHtml(g)}</td>
      <td><button class="btn sec small" type="button" data-g="${esc(g.name)}" onclick="showDetail(this.dataset.g)">Details</button></td>`;
    tb.appendChild(tr);
  });

  updateSelInfo();
}

// ── CHECKBOX SELECTION ─────────────────────────────────────
// selectedGuilds ist die eigentliche Quelle der Wahrheit für die Auswahl
// (übersteht renderTable()-Neuaufbauten, z.B. nach einer Simulation); die
// Checkbox-Elemente selbst sind nur die aktuelle visuelle Darstellung davon.
function toggleGuildSelection(cb){
  if(cb.checked) selectedGuilds.add(cb.dataset.name);
  else selectedGuilds.delete(cb.dataset.name);
  updateSelInfo();
}

// Es gibt zwei Tabellen mit eigenen Checkboxen (Ergebnisse + Vollständige
// Ehrenhalle), die sich denselben selectedGuilds-Auswahltopf teilen — aber
// "Alle auswählen"/"Auswahl aufheben" sollen nur die jeweils EIGENE Tabelle
// betreffen, nicht beide gleichzeitig. Deshalb hier per tbody-Id skopiert.
function scopeEl(scopeId){
  return scopeId ? $(scopeId) : document;
}

function toggleAll(cb, scopeId){
  scopeEl(scopeId).querySelectorAll('.gsel').forEach(c=>{
    c.checked=cb.checked;
    if(cb.checked) selectedGuilds.add(c.dataset.name);
    else selectedGuilds.delete(c.dataset.name);
  });
  updateSelInfo();
}

function selectAllVisible(scopeId){
  scopeEl(scopeId).querySelectorAll('.gsel').forEach(c=>{
    c.checked=true;
    selectedGuilds.add(c.dataset.name);
  });
  updateSelInfo();
}

function deselectAll(scopeId){
  scopeEl(scopeId).querySelectorAll('.gsel').forEach(c=>{
    c.checked=false;
    selectedGuilds.delete(c.dataset.name);
  });
  updateSelInfo();
}

// Zähler-Text ist bewusst über beide Tabellen kombiniert (zeigt die
// gesamte Simulations-Warteschlange); die Header-Checkbox jeder Tabelle
// spiegelt aber nur den Zustand ihrer EIGENEN Zeilen.
function updateSelInfo(){
  const total=document.querySelectorAll('.gsel').length;
  const checked=document.querySelectorAll('.gsel:checked').length;
  const text=`${checked} von ${total} Gilden ausgewählt`;
  $('selInfo').textContent=text;
  const hofInfo=$('selInfoHof'); if(hofInfo) hofInfo.textContent=text;

  syncSelectAllCheckbox('rBody','selAll');
  syncSelectAllCheckbox('hofBody','selAllHof');
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
  const checked=[...document.querySelectorAll('.gsel:checked')].map(c=>c.dataset.name);
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
  const checked=[...document.querySelectorAll('.gsel:checked')].map(c=>c.dataset.name);
  if(!checked.length){
    alert('Keine Gilden ausgewählt.\nBitte zuerst die gewünschten Gilden per Checkbox markieren.');
    return;
  }
  const mushrooms=parseInt($(mushroomSelectId||'simMushrooms').value||'0');

  // Es kann immer nur eine Simulation gleichzeitig laufen, deshalb sind
  // Buttons und Fortschritt für beide Seiten (Ergebnisse + Ehrenhalle) gemeinsam.
  simUi(el=>{el.disabled=true;},'simBtn');
  simUi(el=>{el.disabled=false;},'simAbortBtn');
  simUi(el=>el.classList.remove('hidden'),'simProgress');
  simUi(el=>el.classList.remove('neg'),'simMsg');

  try{
    const r=await api('simulate',{guild_names:checked,mushrooms});
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
      ${g.sim_mushrooms?`Pilzkatapult: <span class="b-gld badge">${g.sim_mushrooms}</span> | `:''}${g.sim_iterations} simulierte Kämpfe | Stand: ${esc(g.sim_evaluated_at||'')}
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
  await api('logout',{});
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

// Nach einem Seiten-Refresh hält der Server die Sitzung noch: Zustand
// wiederherstellen statt wieder den Login zu zeigen.
(async function restore(){
  let st;
  try{
    st=await api('status');
    if(!st.success||!st.data.logged_in)return;
    if(!(st.data.guild_loaded&&st.data.own_guild)){
      const r=await api('characters');
      if(r.success)showChars(r.data);
      return;
    }
    showGuild(st.data.own_guild);
    view='app'; route();
    applyScanStatus(st.data);
  }catch(e){console.error('Wiederherstellung fehlgeschlagen',e);return;}

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
})();

route();
