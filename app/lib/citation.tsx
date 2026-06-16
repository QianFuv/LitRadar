import { Article } from './api';

export type CitationFormat = 'bibtex' | 'ris' | 'apa' | 'mla';

export function generateCitation(article: Article, format: CitationFormat): string {
  const year = article.date ? new Date(article.date).getFullYear() : 'n.d.';
  const authors = article.authors || 'Unknown Author';
  const title = article.title || 'Untitled';
  const journal = article.journal_title || 'Unknown Journal';
  const vol = article.volume || '';
  const num = article.number || '';
  const doi = article.doi || '';

  switch (format) {
    case 'bibtex':
      const id = (authors.split(',')[0] || 'ref').trim().toLowerCase() + year;
      return `@article{${id.replace(/\s+/g, '')},
  author = {${authors}},
  title = {${title}},
  journal = {${journal}},
  year = {${year}},
  ${vol ? `volume = {${vol}},` : ''}
  ${num ? `number = {${num}},` : ''}
  ${doi ? `doi = {${doi}},` : ''}
  url = {${doi ? `https://doi.org/${doi}` : ''}}
}`.trim();

    case 'ris':
      return `TY  - JOUR
AU  - ${authors.split(';').join('\nAU  - ')}
TI  - ${title}
JO  - ${journal}
PY  - ${year}
${vol ? `VL  - ${vol}` : ''}
${num ? `IS  - ${num}` : ''}
${doi ? `DO  - ${doi}` : ''}
ER  -`.trim();

    case 'apa':
      return `${authors} (${year}). ${title}. ${journal}${vol ? `, ${vol}` : ''}${num ? `(${num})` : ''}. ${doi ? `https://doi.org/${doi}` : ''}`.trim();

    case 'mla':
      return `${authors}. "${title}." ${journal}${vol ? `, vol. ${vol}` : ''}${num ? `, no. ${num}` : ''}, ${year}. ${doi ? `doi:${doi}` : ''}`.trim();

    default:
      return '';
  }
}

export function downloadCitation(content: string, filename: string) {
  const blob = new Blob([content], { type: 'text/plain' });
  const url = window.URL.createObjectURL(blob);
  const a = document.createElement('a');
  a.href = url;
  a.download = filename;
  a.click();
  window.URL.revokeObjectURL(url);
}
