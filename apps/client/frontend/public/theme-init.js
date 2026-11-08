/**
 * Apply the stored next-themes preference before the first paint.
 *
 * Mirrors the ThemeProvider configuration in src/providers.tsx: class attribute,
 * "theme" storage key, system default, and color-scheme synchronization.
 */
(function applyInitialTheme() {
  var root = document.documentElement;
  var theme = 'system';
  try {
    theme = window.localStorage.getItem('theme') || 'system';
  } catch (error) {
    theme = 'system';
  }
  if (theme === 'system') {
    theme = window.matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light';
  }
  if (theme !== 'light' && theme !== 'dark') {
    return;
  }
  root.classList.remove('light', 'dark');
  root.classList.add(theme);
  root.style.colorScheme = theme;
})();
