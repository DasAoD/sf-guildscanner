(function () {
  const lang = 'de';
  const t = (key) => (window.I18N[lang] && window.I18N[lang][key]) || key;

  document.querySelectorAll('[data-i18n]').forEach((el) => {
    el.textContent = t(el.dataset.i18n);
  });

  const side = document.getElementById('side');
  const shell = document.getElementById('shell');
  const login = document.getElementById('login');

  function show() {
    const page = (location.hash || '#overview').slice(1);
    const isLogin = page === 'login';
    login.hidden = !isLogin;
    shell.hidden = isLogin;
    if (isLogin) return;

    document.querySelectorAll('.page').forEach((p) => p.classList.toggle('show', p.id === 'p-' + page));
    document.querySelectorAll('.nbtn[data-page]').forEach((b) => b.classList.toggle('active', b.dataset.page === page));
    side.classList.remove('open');
    window.scrollTo(0, 0);
  }

  document.getElementById('menu').addEventListener('click', () => side.classList.toggle('open'));
  document.querySelectorAll('.selall').forEach((box) => {
    box.addEventListener('change', () => {
      box.closest('table').querySelectorAll('tbody input[type=checkbox]').forEach((c) => { c.checked = box.checked; });
    });
  });
  document.querySelectorAll('.char input').forEach((box) => {
    box.addEventListener('change', () => box.closest('.char').classList.toggle('on', box.checked));
  });

  function storeGet(key) {
    try { return JSON.parse(localStorage.getItem(key)) || []; } catch (e) { return []; }
  }
  function storeSet(key, val) {
    try { localStorage.setItem(key, JSON.stringify(val)); } catch (e) { /* ohne Speicher: nur für diese Sitzung */ }
  }

  document.querySelectorAll('.colmenu').forEach((box) => {
    const table = document.getElementById(box.dataset.for);
    const menu = box.querySelector('.menu');
    const heads = Array.from(table.querySelectorAll('thead th'));
    const key = 'cols:' + table.id;
    const hidden = new Set(storeGet(key));

    function apply() {
      heads.forEach((th, i) => {
        const off = hidden.has(th.dataset.col);
        table.querySelectorAll('tr').forEach((tr) => tr.cells[i].classList.toggle('colhide', off));
      });
    }

    heads.filter((th) => !('lock' in th.dataset)).forEach((th) => {
      const label = document.createElement('label');
      label.className = 'check';
      const cb = document.createElement('input');
      cb.type = 'checkbox';
      cb.checked = !hidden.has(th.dataset.col);
      cb.addEventListener('change', () => {
        if (cb.checked) hidden.delete(th.dataset.col); else hidden.add(th.dataset.col);
        storeSet(key, Array.from(hidden));
        apply();
      });
      label.append(cb, th.textContent);
      menu.append(label);
    });

    box.querySelector('button').addEventListener('click', (e) => {
      e.stopPropagation();
      menu.hidden = !menu.hidden;
    });
    menu.addEventListener('click', (e) => e.stopPropagation());
    apply();
  });
  document.addEventListener('click', () => {
    document.querySelectorAll('.colmenu .menu').forEach((m) => { m.hidden = true; });
  });

  window.addEventListener('hashchange', show);
  show();
})();
