import { execSync } from 'node:child_process';

/** Git describe --match pattern for release tags (vMAJOR.MINOR). */
export const VERSION_TAG_MATCH = 'v*';

export function getHeadCommit() {
  return execSync('git rev-parse HEAD', { encoding: 'utf-8', stdio: ['pipe', 'pipe', 'ignore'] }).trim();
}

export function getLatestTag() {
  try {
    return execSync(`git describe --tags --abbrev=0 --match "${VERSION_TAG_MATCH}"`, {
      encoding: 'utf-8',
      stdio: ['pipe', 'pipe', 'ignore'],
    }).trim();
  } catch {
    return null;
  }
}

export function getExactTagAtHead() {
  try {
    return execSync(`git describe --tags --exact-match --match "${VERSION_TAG_MATCH}"`, {
      encoding: 'utf-8',
      stdio: ['pipe', 'pipe', 'ignore'],
    }).trim();
  } catch {
    return null;
  }
}

export function getNextVersion() {
  const latestTag = getLatestTag();
  if (!latestTag) {
    return 'v1.0';
  }

  const match = latestTag.match(/^v(\d+)\.(\d+)$/);
  if (match) {
    const major = parseInt(match[1], 10);
    const minor = parseInt(match[2], 10);
    return `v${major}.${minor + 1}`;
  }

  return 'v1.0';
}

export function createAnnotatedTag(version) {
  const currentTag = getExactTagAtHead();
  if (currentTag === version) {
    console.log(`Current commit is already tagged as ${currentTag}`);
    return;
  }

  console.log(`Creating new tag: ${version}`);
  try {
    execSync(`git tag -a ${version} -m "Release ${version}"`);
  } catch (err) {
    console.error(`Failed to create tag ${version}. It may already exist on another commit.`);
    throw err;
  }
}

export function getAndTagVersion() {
  const currentTag = getExactTagAtHead();
  if (currentTag && currentTag.startsWith('v')) {
    console.log(`Current commit is already tagged as ${currentTag}`);
    return currentTag;
  }

  const newVersion = getNextVersion();
  createAnnotatedTag(newVersion);
  return newVersion;
}
