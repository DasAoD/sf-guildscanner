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

  window.addEventListener('hashchange', show);
  show();
})();
