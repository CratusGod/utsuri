# Utsuri end-to-end test — 56/63 passed

| # | Where | Scenario | Result | Detail | Time |
|---|---|---|---|---|---|
| 1 | explorer | JPG → PNG | ✅ | Photo.png · 2400×1600 | 7.6s |
| 2 | explorer | JPG → WEBP | ✅ | Photo.webp · 2400×1600 | 5.3s |
| 3 | explorer | JPG → HEIC | ✅ | Photo.heic · 2400×1600 | 5.9s |
| 4 | explorer | JPG → TIFF | ✅ | Photo.tiff · 2400×1600 | 5.0s |
| 5 | explorer | JPG → AVIF | ✅ | Photo.avif · 2400×1600 | 5.5s |
| 6 | explorer | JPG → BMP | ✅ | Photo.bmp · 2400×1600 | 5.1s |
| 7 | explorer | JPG → PDF | ✅ | Photo.pdf · 1 page(s) | 5.1s |
| 8 | explorer | JPG → ICO | ✅ | Photo.ico · 128×86 | 5.0s |
| 9 | explorer | JPG → GIF | ✅ | Photo.gif · 2400×1600 | 5.3s |
| 10 | explorer | PNG → JPG | ✅ | Graphic.jpg · 800×533 | 5.0s |
| 11 | explorer | PDF → JPG (3 pages) | ✅ | 3 × .jpg | 5.1s |
| 12 | explorer | PDF → PNG (3 pages) | ✅ | 3 × .png | 6.4s |
| 13 | explorer | PDF → DOCX | ✅ | Report.docx | 5.0s |
| 14 | explorer | PDF → TXT | ✅ | Report.txt | 5.0s |
| 15 | explorer | DOCX → PDF | ✅ | Letter.pdf · 1 page(s) | 5.0s |
| 16 | explorer | DOCX → TXT | ✅ | Letter.txt | 5.0s |
| 17 | explorer | TXT → PDF | ✅ | Notes.pdf · 1 page(s) | 5.0s |
| 18 | explorer | TXT → DOCX | ✅ | Notes.docx | 5.0s |
| 19 | explorer | 2 images → 1 PDF | ✅ | 2 images → 2-page PDF | 6.5s |
| 20 | explorer | Compress to 200 KB | ✅ | 354 KB → 184 KB (48% smaller) | 10.4s |
| 21 | explorer | Resize 50% | ✅ | 2400×1600 → 1200×800 | 8.3s |
| 22 | explorer | Crop 1:1 | ✅ | 2400×1600 → 1600×1600 | 8.8s |
| 23 | explorer | Edit: auto + rotate | ✅ | 2400×1600 → 1600×2400 | 9.8s |
| 24 | explorer | Annotate | ✅ | Photo Annotated.jpg | 10.9s |
| 25 | explorer | Redact | ✅ | Photo Redacted.jpg · centre blacked out | 9.5s |
| 26 | explorer | Add BG 1:1 | ✅ | 2400×1600 → 2976×2976 | 10.8s |
| 27 | explorer | Metadata: remove location | ✅ | GPS removed, kept Apple iPhone 15 Pro | 7.6s |
| 28 | explorer | Metadata: remove all | ✅ | 0 metadata fields left | 7.7s |
| 29 | explorer | PDF Compress | ✅ | 227 KB → 108 KB | 6.2s |
| 30 | explorer | PDF Split | ✅ | 3 PDFs | 6.2s |
| 31 | explorer | PDF Rotate | ✅ | all pages at 90° | 6.1s |
| 32 | explorer | PDF Protect | ✅ | AES-256, password works | 9.3s |
| 33 | explorer | PDF Extract images | ✅ | 1 image(s) in Report images/ | 6.2s |
| 34 | explorer | PDF Metadata | ✅ | author/title removed | 7.6s |
| 35 | explorer | Merge 2 PDFs | ✅ | 5 pages | 7.8s |
| 36 | explorer | More: Copy image | ✅ | clipboard image 1200×900 | 8.9s |
| 37 | explorer | More: Text (OCR) | ❌ | ValueError: 'Text' is not in list | 0.7s |
| 38 | explorer | More: PDF Gray | ✅ | Report Gray.pdf · grayscale | 7.2s |
| 39 | explorer | More: Stamp in Vietnamese | ✅ | Report Watermarked.pdf · on all 3 pages · Vietnamese letters from Segoe UI Regular | 20.0s |
| 40 | explorer | Pack: ZIP | ✅ | Photo.zip holds Photo.jpg | 8.3s |
| 41 | explorer | Pack: TAR.GZ | ✅ | Notes.tar.gz holds Notes.txt | 7.2s |
| 42 | explorer | Extract archive, then Undo | ✅ | extracted Graphic.png, readme.txt into Bundle/ · Undo removed Bundle/ | 7.4s |
| 43 | explorer | More: Translate in Oumu | ✅ | Oumu got the text (31 chars) | 7.8s |
| 44 | explorer | Vietnamese name: JPG → PNG | ✅ | Ảnh chụp.png · 900×600 | 5.0s |
| 45 | explorer | 3 files → one ZIP | ✅ | Archive.zip holds Graphic.png, Letter.docx, Notes.txt | 10.6s |
| 46 | explorer | Convert, then Undo | ✅ | Second.webp · 1200×900 · Undo removed Second.webp | 6.7s |
| 47 | explorer | Video → GIF | ❌ | KeyError: 'Convert' | 0.7s |
| 48 | explorer | Video → WEBM | ❌ | KeyError: 'Convert' | 0.7s |
| 49 | explorer | Video → MP3 | ❌ | KeyError: 'Convert' | 0.7s |
| 50 | explorer | Video Compress | ❌ | KeyError: 'More' | 0.7s |
| 51 | send-to | Send To → PNG | ✅ | Photo.png · 2400×1600 | 5.3s |
| 52 | desktop | Desktop: JPG → PNG | ✅ | D-Photo.png · 2400×1600 | 5.5s |
| 53 | desktop | Desktop: PDF → TXT | ✅ | D-Report.txt | 4.9s |
| 54 | desktop | Desktop: DOCX → PDF | ✅ | D-Letter.pdf · 1 page(s) | 4.9s |
| 55 | desktop | Desktop: Compress | ✅ | 354 KB → 184 KB (48% smaller) | 9.9s |
| 56 | desktop | Desktop: Crop | ✅ | 2400×1600 → 1600×1600 | 8.6s |
| 57 | desktop | Desktop: Add BG | ✅ | 800×533 → 992×992 | 9.2s |
| 58 | desktop | Desktop: 2 PDFs merge | ✅ | 5 pages | 7.2s |
| 59 | desktop | Desktop: Pack ZIP | ✅ | D-Photo.zip holds D-Photo.jpg | 8.1s |
| 60 | desktop | Desktop: Extract, then Undo | ❌ | AssertionError: Bundle still there after Undo | 15.4s |
| 61 | desktop | Desktop: Video → GIF | ❌ | KeyError: 'Convert' | 0.7s |
| 62 | explorer | Colour themes | ✅ | 8 themes, all distinct (closest: Catppuccin Macchiato / Tokyo Night Storm, 1.9) · shots/theme_*.png | 80.3s |
| 63 | all | Drag picture kept out of the glass | ✅ | hidden during 59 menu captures | 0s |