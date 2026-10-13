// The native server owns authentication, browser context and collection state.
const feedback = document.getElementById('feedback');
function say(message, error = false) {
  feedback.setAttribute('role', error ? 'alert' : 'status');
  feedback.textContent = message;
}
async function replyMessage(response) {
  const text = await response.text();
  if ((response.headers.get('Content-Type') || '').includes('text/html')) {
    return new DOMParser().parseFromString(text, 'text/html').getElementById('request-error')?.textContent || 'The café could not complete this request. Try again.';
  }
  return text;
}
async function csrf() {
  const response = await fetch('/identity/csrf', { credentials: 'same-origin', cache: 'no-store' });
  if (!response.ok) throw new Error('Could not prepare a secure request. Reload and try again.');
  return (await response.json()).token;
}
const login = document.getElementById('login');
if (login) login.addEventListener('submit', async event => {
  event.preventDefault();
  const button = document.getElementById('sign-in');
  if (button.getAttribute('aria-disabled') === 'true') return;
  button.setAttribute('aria-disabled', 'true');
  login.setAttribute('aria-busy', 'true');
  document.getElementById('username').removeAttribute('aria-invalid');
  document.getElementById('password').removeAttribute('aria-invalid');
  say('Signing in…');
  try {
    const token = await csrf();
    const fields = new FormData(login);
    const response = await fetch(login.action, {
      method: 'POST', credentials: 'same-origin',
      headers: { 'Content-Type': 'application/json', 'X-Identity-CSRF': token },
      body: JSON.stringify({ username: fields.get('username'), password: fields.get('password') })
    });
    const message = await replyMessage(response);
    if (!response.ok) {
      const error = new Error(message);
      error.invalidCredentials = response.status === 401 || (response.status === 400 && message === 'Invalid login fields. Enter a username and password.');
      throw error;
    }
    window.location.assign('/pickup');
  } catch (error) {
    say(error.message || 'The café could not sign you in. Try again.', true);
    if (error.invalidCredentials) {
      document.getElementById('username').setAttribute('aria-invalid', 'true');
      document.getElementById('password').setAttribute('aria-invalid', 'true');
      document.getElementById('password').focus();
    } else {
      button.focus();
    }
  } finally {
    button.removeAttribute('aria-disabled');
    login.removeAttribute('aria-busy');
  }
});
const collect = document.getElementById('collect');
if (collect) collect.addEventListener('submit', async event => {
  event.preventDefault();
  const button = document.getElementById('collect-order');
  if (button.getAttribute('aria-disabled') === 'true') return;
  button.setAttribute('aria-disabled', 'true');
  collect.setAttribute('aria-busy', 'true');
  say('Recording your collection…');
  try {
    const token = await csrf();
    const response = await fetch(collect.action, {
      method: 'POST', credentials: 'same-origin', headers: { 'X-Identity-CSRF': token }
    });
    const message = await replyMessage(response);
    if (!response.ok) throw new Error(message);
    document.getElementById('order-state').textContent = 'Collected';
    document.getElementById('received').disabled = true;
    button.textContent = 'Collection recorded';
    button.setAttribute('aria-disabled', 'true');
    say(message);
  } catch (error) {
    button.removeAttribute('aria-disabled');
    say(error.message || 'The café could not record collection. Try again.', true);
    button.focus();
  } finally {
    collect.removeAttribute('aria-busy');
  }
});
