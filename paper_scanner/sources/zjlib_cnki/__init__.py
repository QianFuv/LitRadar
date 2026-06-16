"""Zhejiang Library mediated CNKI full-text integration utilities."""

from paper_scanner.sources.zjlib_cnki.client import (
    ClientInfo,
    CnkiArticleCandidate,
    DownloadedPdf,
    QrLogin,
    SearchResult,
    ZhejiangLibraryCnkiClient,
    ZjlibCnkiError,
)
from paper_scanner.sources.zjlib_cnki.matching import (
    ArticleIdentity,
    does_article_metadata_match,
)

__all__ = [
    "ArticleIdentity",
    "ClientInfo",
    "CnkiArticleCandidate",
    "DownloadedPdf",
    "QrLogin",
    "SearchResult",
    "ZhejiangLibraryCnkiClient",
    "ZjlibCnkiError",
    "does_article_metadata_match",
]
