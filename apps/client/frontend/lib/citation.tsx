/**
 * Safe DOI and external-link helpers.
 */

/** Recognizes a DOI value without a resolver URL. */
const DOI_VALUE_PATTERN = /^10\.\d{4,9}\/\S+$/iu;

/** Recognizes an explicit URI scheme before a value is treated as a bare DOI. */
const URI_SCHEME_PATTERN = /^[a-z][a-z\d+.-]*:/iu;

/**
 * Validate and normalize an absolute HTTP(S) URL.
 *
 * @param value - Candidate external URL.
 * @returns Normalized safe URL or null for relative, malformed, or unsafe schemes.
 */
export function getSafeHttpUrl(value: string | null | undefined): string | null {
  const normalizedValue = value?.trim();
  if (!normalizedValue) {
    return null;
  }
  try {
    const url = new URL(normalizedValue);
    return url.protocol === 'http:' || url.protocol === 'https:' ? url.toString() : null;
  } catch {
    return null;
  }
}

/**
 * Normalize a DOI from a raw value, DOI prefix, or doi.org resolver URL.
 *
 * @param value - Candidate DOI value.
 * @returns Bare DOI or null when the value is not a DOI.
 */
function normalizeDoiValue(value: string | null | undefined): string | null {
  let normalizedValue = value?.trim();
  if (!normalizedValue) {
    return null;
  }
  normalizedValue = normalizedValue.replace(/^doi:\s*/iu, '');

  const safeUrl = getSafeHttpUrl(normalizedValue);
  if (safeUrl) {
    const url = new URL(safeUrl);
    if (url.hostname !== 'doi.org' && url.hostname !== 'dx.doi.org') {
      return null;
    }
    try {
      normalizedValue = decodeURIComponent(url.pathname.replace(/^\//u, ''));
    } catch {
      return null;
    }
  }

  return DOI_VALUE_PATTERN.test(normalizedValue) ? normalizedValue : null;
}

/**
 * Resolve a DOI field to a safe clickable destination.
 *
 * @param value - Raw DOI field.
 * @returns Safe HTTP(S) resolver URL or null.
 */
export function getDoiUrl(value: string | null | undefined): string | null {
  const normalizedValue = value?.trim();
  if (!normalizedValue) {
    return null;
  }

  const explicitUrl = getSafeHttpUrl(normalizedValue);
  if (explicitUrl) {
    return explicitUrl;
  }
  if (URI_SCHEME_PATTERN.test(normalizedValue) && !/^doi:/iu.test(normalizedValue)) {
    return null;
  }

  const doi = normalizeDoiValue(normalizedValue);
  return doi ? `https://doi.org/${doi}` : null;
}
