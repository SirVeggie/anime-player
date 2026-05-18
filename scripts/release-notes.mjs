import fs from 'node:fs/promises';
import path from 'node:path';
import { execSync } from 'node:child_process';
import { getExactTagAtHead, getNextVersion, VERSION_TAG_MATCH } from './release-version.mjs';

function describeVersionTag(ref) {
  return execSync(`git describe --tags --abbrev=0 --match "${VERSION_TAG_MATCH}" ${ref}`, {
    encoding: 'utf-8',
    stdio: ['pipe', 'pipe', 'ignore'],
  }).trim();
}

async function generateReleaseNotes() {
  const root = process.cwd();
  const releasesDir = path.join(root, 'releases');

  try {
    await fs.access(releasesDir);
  } catch {
    await fs.mkdir(releasesDir, { recursive: true });
  }

  let targetVersion = getExactTagAtHead();
  let commitRange = '';

  if (targetVersion) {
    let previousTag = '';
    try {
      previousTag = describeVersionTag(`${targetVersion}~1`);
    } catch {
      // No previous tag
    }

    if (previousTag) {
      commitRange = `${previousTag}..HEAD`;
    } else {
      commitRange = 'HEAD';
    }
  } else {
    targetVersion = getNextVersion();
    let latestTag = '';
    try {
      latestTag = describeVersionTag('HEAD');
    } catch {
      // No latest tag
    }

    if (latestTag) {
      commitRange = `${latestTag}..HEAD`;
    } else {
      commitRange = 'HEAD';
    }
  }

  console.log(`Generating notes for ${targetVersion} from commits: ${commitRange}`);

  let commits = '';
  try {
    commits = execSync(`git log ${commitRange} --no-merges --pretty=format:"- %s (%h)"`, { encoding: 'utf-8' }).trim();
  } catch (err) {
    console.error('Failed to get git log:', err.message);
    process.exit(1);
  }

  const filteredCommits = commits.split('\n')
    .filter(line => line.trim().length > 0)
    .filter(line => !line.match(/^- (Merge|checkpoint)/i))
    .join('\n');

  const notesContent = `# Anime Player ${targetVersion}

## Changes

${filteredCommits || '- No significant changes logged.'}
`;

  const notesPath = path.join(releasesDir, `NOTES-${targetVersion}.md`);
  await fs.writeFile(notesPath, notesContent, 'utf8');

  console.log(`\nDraft release notes written to:`);
  console.log(notesPath);
  console.log(`\nReview and polish this file before running 'npm run release:publish'.`);
}

generateReleaseNotes().catch((err) => {
  console.error(err);
  process.exit(1);
});
