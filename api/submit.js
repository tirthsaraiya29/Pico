// Pico website issue submission endpoint (Vercel serverless / Node.js).
// Creates public GitHub issues for bug/feature/appreciation submissions and
// stores contact details privately. Security vulnerability reports never become public issues.
//
// Required environment variables (set in Vercel project settings, never committed):
//   GITHUB_TOKEN        — fine-grained token with "Issues: Read & write" on tirthsaraiya29/Pico
//   PRIVATE_REPO_TOKEN  — (optional) token with repo scope on private contacts repository;
//                         falls back to GITHUB_TOKEN if unset
//   CONTACTS_REPO       — (optional) private repo for contact storage,
//                         default "tirthsaraiya29/pico-submissions" (must be private)

const REPO = 'tirthsaraiya29/Pico';
const API = 'https://api.github.com';

const MAX_FIELD = 10000;
const MAX_BODY = 200000;

const LABELS = {
  bug: { title: 'Bug', label: 'bug' },
  feature: { title: 'Feature', label: 'enhancement' },
  appreciation: { title: 'Appreciation', label: 'appreciation' }
};

// In-memory per-IP rate limiter (per serverless instance)
const rateMap = new Map();
const RATE_LIMIT = 5;
const RATE_WINDOW_MS = 10 * 60 * 1000; // 10 minutes

function json(res, status, obj) {
  res.statusCode = status;
  res.setHeader('Content-Type', 'application/json');
  res.setHeader('Access-Control-Allow-Origin', '*');
  res.setHeader('Access-Control-Allow-Headers', 'Content-Type');
  res.setHeader('Access-Control-Allow-Methods', 'POST, OPTIONS');
  res.end(JSON.stringify(obj));
}

function cleanStr(v, max = MAX_FIELD) {
  if (typeof v !== 'string') return '';
  return v.replace(/[\u0000-\u0008\u000B\u000C\u000E-\u001F\u007F]/g, '').trim().slice(0, max);
}

function mdEsc(s) {
  // Prevent Markdown/HTML injection into issue bodies
  return String(s == null ? '' : s).replace(/^#/gm, '#​').replace(/</g, '&lt;');
}

function checkRate(ip) {
  const now = Date.now();
  const rec = rateMap.get(ip);
  if (!rec || now - rec.start > RATE_WINDOW_MS) {
    rateMap.set(ip, { start: now, n: 1 });
    return { ok: true };
  }
  rec.n += 1;
  if (rec.n > RATE_LIMIT) {
    return { ok: false, retry: Math.ceil((RATE_WINDOW_MS - (now - rec.start)) / 1000) };
  }
  return { ok: true };
}

function ghHeaders(token) {
  return {
    'Authorization': `Bearer ${token}`,
    'Accept': 'application/vnd.github+json',
    'User-Agent': 'pico-website'
  };
}

function buildBody(category, f, credit) {
  const creditLine = credit ? '<sub>Reported by [@' + credit + '](https://github.com/' + credit + ').</sub>\n' : '';
  const lines = [];

  if (category === 'bug') {
    lines.push('### Affected component', f.component || '_not provided_', '');
    lines.push('### Describe the bug', mdEsc(f.description), '');
    lines.push('### Steps to reproduce', mdEsc(f.reproduce), '');
    lines.push('### Expected behaviour', mdEsc(f.expected), '');
    lines.push('### Actual behaviour', mdEsc(f.actual), '');
    lines.push('### Pico version or commit', f.version || '_not provided_', '');
    lines.push('### Operating System & Hardware', f.os_env || '_not provided_', '');
    lines.push('### Audio environment', f.audio_env || '_not provided_', '');
    lines.push('### Logs and terminal output', mdEsc(f.logs || '_not provided_'), '');
    lines.push('### Additional context', mdEsc(f.additional || '_not provided_'), '');
    if (creditLine) lines.push(creditLine);
    lines.push('_Submitted via the Pico website issue submission portal._');
  } else if (category === 'feature') {
    lines.push('### Problem or use case', mdEsc(f.problem), '');
    lines.push('### Proposed solution', mdEsc(f.solution), '');
    lines.push('### Relevant component', f.component || '_not provided_', '');
    lines.push('### Alternatives considered', mdEsc(f.alternatives || '_not provided_'), '');
    lines.push('### Additional context', mdEsc(f.additional || '_not provided_'), '');
    if (creditLine) lines.push(creditLine);
    lines.push('_Submitted via the Pico website issue submission portal._');
  } else {
    lines.push('### Your message', mdEsc(f.message), '');
    lines.push('### Suggestions or feedback', mdEsc(f.feedback || '_not provided_'), '');
    lines.push('### Acknowledgements', f.ack ? '- [x] No sensitive passwords, private keys, tokens, or confidential data included.' : '- [ ] (missing)', '');
    if (creditLine) lines.push(creditLine);
    lines.push('_Submitted via the Pico website issue submission portal._');
  }

  return lines.join('\n').slice(0, MAX_BODY);
}

async function storeContact(ref, email, phone, github, githubPublic, token, contactsRepo) {
  if (!token || !contactsRepo) return;
  const content = Buffer.from(
    JSON.stringify({
      ref,
      email: email || null,
      phone: phone || null,
      github: github || null,
      github_public: !!githubPublic,
      at: new Date().toISOString()
    }, null, 2)
  ).toString('base64');

  const res = await fetch(`${API}/repos/${contactsRepo}/contents/${ref}.json`, {
    method: 'PUT',
    headers: { ...ghHeaders(token), 'Content-Type': 'application/json' },
    body: JSON.stringify({
      message: `Contact info for submission ${ref}`,
      content,
      branch: 'contacts'
    })
  });
  if (!res.ok && res.status !== 422) {
    throw new Error(`contact store failed: ${res.status}`);
  }
}

module.exports = async (req, res) => {
  if (req.method === 'OPTIONS') {
    res.statusCode = 204;
    res.setHeader('Access-Control-Allow-Origin', '*');
    res.setHeader('Access-Control-Allow-Headers', 'Content-Type');
    res.setHeader('Access-Control-Allow-Methods', 'POST, OPTIONS');
    return res.end();
  }

  if (req.method !== 'POST') {
    return json(res, 405, { ok: false, error: 'Method not allowed' });
  }

  const token = process.env.GITHUB_TOKEN;
  if (!token) {
    return json(res, 503, {
      ok: false,
      error: 'Submission service is not configured with GITHUB_TOKEN. Please submit issues directly on GitHub at https://github.com/tirthsaraiya29/Pico/issues.'
    });
  }

  const ip = (req.headers['x-forwarded-for'] || req.socket.remoteAddress || 'unknown').split(',')[0].trim();
  const rate = checkRate(ip);
  if (!rate.ok) {
    return json(res, 429, { ok: false, error: 'Too many submissions. Please wait before trying again.', retry_after: rate.retry });
  }

  let body;
  try {
    const raw = await new Promise((resolve, reject) => {
      let data = '';
      req.on('data', c => {
        data += c;
        if (data.length > MAX_BODY) { reject(new Error('payload too large')); req.destroy(); }
      });
      req.on('end', () => resolve(data));
      req.on('error', reject);
    });
    body = JSON.parse(raw);
  } catch (e) {
    return json(res, 400, { ok: false, error: 'Invalid JSON request payload.' });
  }

  const category = cleanStr(body.category, 20);
  const honeypot = cleanStr(body.website, 100);
  if (honeypot) {
    // Silent drop for bot spam
    return json(res, 200, { ok: true });
  }

  if (body._ts && Date.now() - Number(body._ts) < 2000) {
    return json(res, 400, { ok: false, error: 'Submission rejected: submitted too quickly (bot check).' });
  }

  const fields = body.fields && typeof body.fields === 'object' ? body.fields : {};
  const f = {};
  for (const k of Object.keys(fields)) f[cleanStr(k, 40)] = cleanStr(fields[k]);

  const email = cleanStr(body.contact_email, 200);
  const phone = cleanStr(body.contact_phone, 30);
  const github = cleanStr(body.contact_github, 40);
  const githubPublic = github && body.contact_github_public === true;

  if (!email && !phone) {
    return json(res, 400, { ok: false, error: 'Contact information (email or phone) is required so we can follow up.' });
  }

  if (github && !/^[a-zA-Z\d](?:[a-zA-Z\d]|-(?=[a-zA-Z\d])){0,38}$/.test(github)) {
    return json(res, 400, { ok: false, error: 'Invalid GitHub username format.' });
  }

  const ref = `sub-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 8)}`;
  const contactsRepo = process.env.CONTACTS_REPO || 'tirthsaraiya29/pico-submissions';

  try {
    // Security vulnerabilities: private only, NEVER a public issue
    if (category === 'security') {
      if (f.ack !== true && f.ack !== 'true') {
        return json(res, 400, { ok: false, error: 'You must acknowledge confidential handling.' });
      }

      await storeContact(ref, email, phone, github, false, process.env.PRIVATE_REPO_TOKEN || token, contactsRepo);

      const detail = [
        `Ref: ${ref}`,
        `Affected component: ${f.component || 'unspecified'}`,
        `Reproducible: ${f.reproducible || 'unspecified'}`,
        '',
        '### Description',
        mdEsc(f.description || 'not provided'),
        '',
        '### Potential Impact',
        mdEsc(f.impact || 'not provided'),
        '',
        '### Reproduction steps / Proof of concept',
        mdEsc(f.reproduce || 'not provided')
      ].join('\n');

      const reportB64 = Buffer.from(detail).toString('base64');
      const saved = await fetch(`${API}/repos/${contactsRepo}/contents/${ref}-report.md`, {
        method: 'PUT',
        headers: { ...ghHeaders(process.env.PRIVATE_REPO_TOKEN || token), 'Content-Type': 'application/json' },
        body: JSON.stringify({ message: `Security report ${ref}`, content: reportB64, branch: 'contacts' })
      });

      if (!saved.ok && saved.status !== 422) {
        throw new Error(`security report store failed: ${saved.status}`);
      }

      return json(res, 200, {
        ok: true,
        reference: ref,
        message: 'Security vulnerability report received privately. Thank you for responsible disclosure.'
      });
    }

    // Public categories: create GitHub issue
    const tpl = LABELS[category];
    if (!tpl) {
      return json(res, 400, { ok: false, error: 'Unknown submission category.' });
    }

    const titleGuess = f.description || f.problem || f.message || '';
    const titleFirstLine = titleGuess.split('\n')[0].slice(0, 80) || ref;
    const title = `[${tpl.title}]: ${titleFirstLine}`;
    const issueBody = buildBody(category, f, githubPublic ? github : null);

    const created = await fetch(`${API}/repos/${REPO}/issues`, {
      method: 'POST',
      headers: { ...ghHeaders(token), 'Content-Type': 'application/json' },
      body: JSON.stringify({ title, body: issueBody, labels: [tpl.label] })
    });

    if (!created.ok) {
      const errText = await created.text();
      console.error(`issue creation failed: ${created.status} ${errText.slice(0, 300)}`);
      return json(res, 502, {
        ok: false,
        error: 'Could not create issue on GitHub. Please submit directly via https://github.com/tirthsaraiya29/Pico/issues.'
      });
    }

    const issue = await created.json();

    // Store private contact info tied to issue number
    try {
      await storeContact(`issue-${issue.number}`, email, phone, github, githubPublic, process.env.PRIVATE_REPO_TOKEN || token, contactsRepo);
    } catch (e) {
      console.error(`contact storage failed for issue ${issue.number}: ${e.message}`);
    }

    return json(res, 200, {
      ok: true,
      number: issue.number,
      url: issue.html_url
    });
  } catch (e) {
    console.error(`submit error: ${e.message}`);
    return json(res, 500, { ok: false, error: 'Internal error processing submission. Please try again.' });
  }
};
