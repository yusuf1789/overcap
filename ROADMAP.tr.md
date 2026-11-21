# Overcap — Geliştirme Fikirleri

[English version: ROADMAP.md](ROADMAP.md) · [Türkçe README](README.tr.md)

Bu belge, günlük kullanım ve oyun deneyimini iyileştirmek için önerilen geliştirmeleri kaydeder. Uygulanmış maddeler ayrıca işaretlenmiştir; diğerleri öneridir ve ayrıntıları geliştirme sırasında netleştirilecektir.

Öncelik, mevcut doğal çeviri kalitesini koruyarak kullanım zahmetini azaltmak ve doğru çeviriyi doğru anda göstermektir. Tasarım oyun ve uygulama bağımsız kalmalıdır.

## 1. Altyazı Bölgesini Pencereye Bağlamak

**Önerilen öncelik: İlk aşama.**

Şu an seçilen bölge ekran koordinatlarıyla saklanır. Kaynak pencere taşındığında veya boyutu değiştiğinde bölge eski yerde kalabilir. Kullanıcı, seçtiği alanı Remote Play, video oynatıcı veya başka bir kaynak pencereye bağlayabilmelidir.

Önerilen davranış:

- Mevcut sabit ekran bölgesi seçeneği korunur; pencere takibi isteğe bağlıdır.
- Kullanıcı kaynak pencereyi açıkça seçer. Kaynak, yalnızca odaktaki uygulama değişti diye başka bir pencereye dönüşmez.
- Seçili alan, pencerenin başlık ve kenarlıklarını dışarıda bırakan içerik alanına göre konum ve boyut oranlarıyla saklanır. Pencere hareketinde yeni ekran koordinatları hesaplanır; yeniden boyutlandırmada bölge orantılı güncellenir.
- Yakalama alanı ile çeviri overlay'i birlikte güncellenir. Koordinat değişimi tek başına aynı replik için yeni bir API isteği gerektirmemelidir.
- Kaynak küçültülür, kapanır veya kullanılabilir bir yakalama alanı kalmazsa OCR duraklatılır ve eski overlay gizlenir. Başka uygulamaların bulunduğu eski koordinatlar okunmaya devam edilmez.
- Pencere yeniden kullanılabilir olduğunda takip devam eder. Uygulama yeniden açıldıysa pencere kimliği yeniden doğrulanır; aynı uygulamanın birden fazla penceresi varsa belirsiz seçim otomatik yapılmaz.

Teknik kapsam: `selector.rs` ve `profiles.rs` ile kaynak pencereyi seçme/kimliğini doğrulama; `config.rs` ile geriye uyumlu isteğe bağlı bağlama bilgisi; `main.rs` ile çalışma anında koordinat takibi; `overlay.rs` ile kaynak sınırlarını ve görünürlüğü güncelleme; `gui.rs` ile takip seçeneği ve kaynak durumu. Sık konum güncellemeleri her seferinde ayar dosyasına yazılmaz.

**Sınırlar:** Pencere boyutunun orantılı değişmesi, video içeriğinin her zaman aynı oranda yerleştiği anlamına gelmez. Siyah bantlar, en-boy oranı veya oynatıcı arayüzü değiştiğinde yeniden bölge seçimi gerekebilir. İlk sürümde kusursuz otomatik video alanı tespiti vaat edilmez. Mevcut GDI ekran yakalaması korunursa kaynak içeriğin ekranda görünür olması gerekir; pencere takibi, arkada kalan veya küçültülmüş pencereyi yakalama özelliği değildir.

**Kabul ölçütleri:** Taşıma, yeniden boyutlandırma, farklı DPI'lı monitörler arasında geçiş, küçültme/geri getirme ve kapatma/yeniden açma senaryoları doğrulanır. Sabit ekran bölgesi, odak çalmama, tıklama geçirgenliği ve `WDA_EXCLUDEFROMCAPTURE` davranışı korunur. Tam ekran geçişi kaynak pencereyi veya içerik yerleşimini değiştirebileceği için ayrıca sınanır.

## 2. Hızlı Diyaloglarda Replik Senkronizasyonu

**Önerilen öncelik: İlk aşama.**

Yeni bir replik ekrana geldikten sonra eski bir çeviri isteği tamamlanabilir. Amaç, en son tamamlanan isteği göstermek yerine ekrandaki kaynak replikle eşleşen çeviriyi sunmaktır.

- Sonuçları kaynak replik kimliği ve güncelliğiyle ilişkilendirmek.
- Yeni repliğe geçildiğinde eski sonucun geç görünmesini ve önceki repliğe geri dönüşü engellemek.
- Kaynak metin kaybolduğunda çevirinin gizlenme zamanını iyileştirmek; kısa OCR boşluklarında titremeyi önlemek.
- Önceki satırlardan gelen çeviri bağlamını ve kısa süreli OCR hatalarına toleransı korumak.

**Kabul ölçütü:** Hızlı konuşma ve ani altyazı değişimlerinde yanlış repliğin gösterilmemesi; uzun repliklerin gereksiz yere kaybolmaması. Başarı yalnızca toplam milisaniyeyle değil, doğru repliğin görünme süresiyle değerlendirilir.

## 3. Daha Kapsamlı Uygulama Profilleri

Bölge ve görünüm profillerine kaynak dil, hedef dil ve çeviri motoru/model tercihi eklemek. Örneğin Remote Play ve video oynatıcı için farklı tercihler otomatik uygulanabilir.

**Kabul ölçütü:** Profil değişiminde doğru ayarların tutarlı uygulanması, eski sonuçların yeni profil altında gösterilmemesi ve mevcut ayar dosyalarının sorunsuz yüklenmesi. API anahtarları profillere gereksiz yere çoğaltılmaz.

## 4. İsteğe Bağlı Terim ve İsim Sözlüğü

Karakter isimleri, yerler ve özel terimler için kullanıcı tarafından belirlenen karşılıkları veya “çevirmeden koru” tercihlerini kullanmak. Sözlük genel veya uygulama profiline bağlı olabilir.

**Kabul ölçütü:** Uzun oturumlarda isim ve terim tutarlılığının artması; doğal diyalog kalitesinin korunması. Sözlük, sözcüklerin içini veya farklı anlamdaki kullanımları körlemesine değiştiren bir metin değiştirme işlemi olmamalıdır. Ek gecikme ve motorlar arasındaki destek farkı ölçülmelidir.

## 5. Oyun İçinde Kısa Çeviri Geçmişi

**Durum: v0.4.4 ile uygulandı.** Son 15 hedef çeviri, `Ctrl+Shift+L` ile açılan odak almayan panelde gösterilir. Fare tekerleği ve panel açıkken `Ctrl+Shift+PageUp/PageDown` ile kaydırılır. Oturumluk geçmiş diske kaydedilmez; EN/TR arayüz dilini izler. Kullanım için [README](README.tr.md#oyun-içi-çeviri-geçmişi).

Bir kısayolla son birkaç repliği gösteren geçici bir geçmiş görünümü açmak. Kullanıcı kaçırdığı repliği ana kontrol penceresine dönmeden okuyabilmelidir.

**Kabul ölçütü:** Görünüm oyundan odak çalmadan açılıp kapanır; geçmiş boyutu sınırlıdır ve mevcut altyazı akışını engellemez. Geçmiş görünümü de OCR yakalamasından hariç tutulur.

## 6. Uzun Oturum Dayanıklılığı

Ağ kesilip gelmesi, bilgisayarın uykudan dönmesi, çözünürlük/DPI değişimi ve Remote Play'in yeniden açılması gibi durumlarda uygulamanın anlaşılır durum bildirimiyle toparlanmasını sağlamak.

**Kabul ölçütü:** Yeniden denemeler sınırlı ve kontrollüdür; eski replikler birikmez. Kaynak ve motor durumları doğru gösterilir; geçici hata sonrasında uygulamayı yeniden başlatmak gerekmez. Gerçek Remote Play oturumlarında ayrıca denenir.

## 7. Aynı Çeviri İsteğinde Konuşmacı Ayrımı ve Görsel Vurgu

**Durum: Fikir; henüz uygulanmadı.**

Çeviriyi yapan model, aynı istekte konuşmacı etiketini ve çevrilmiş diyaloğu ayrı alanlar halinde döndürebilir. İkinci model çağrısı yapılmaz. Birden fazla konuşmacı varsa sıralı parçalar kullanılır; her parça isteğe bağlı `speaker` ve çevrilmiş `text` alanını taşır.

- Yalnızca kaynak metinde açıkça belirtilen konuşmacı ayrılır; isim veya kimlik tahmin edilmez. Etiket yoksa `speaker` boş/null olur. “Michael, buraya gel!” cümlesindeki hitap konuşmacı etiketi sayılmaz; görev başlıkları gibi belirsiz metinler zorla isim olarak ayrılmaz.
- Modelin ayırdığı isim, çizim aşamasında farklı renk ve kalınlıkla gösterilir; diyalog mevcut görünümünü korur. Appearance bölümünde isteğe bağlı konuşmacı vurgusu ve renk seçimi sunulur. İlk sürümde tüm konuşmacılar için tek vurgu rengi yeterlidir.
- Google yedeği veya yapılandırılmış yanıtı desteklemeyen bir motor kullanıldığında normal, tek renk çeviri gösterilir. Bozuk/eksik yapılandırılmış yanıtta kullanılabilir çeviri metni korunur; ham JSON gösterilmez, replik sessizce kaybedilmez ve biçimi düzeltmek için ek ağ isteği yapılmaz.
- Konuşmacı bilgisi önbellekte ve geçmişte de korunur. Destek her motor/model için doğrulanır; yalnızca uyumlu API biçimine sahip olmak yeterli kabul edilmez.
- Ek alanlar üretmek sıfır maliyetli değildir. Mevcut model ve iki satırlık bağlam korunarak aynı repliklerde çeviri kalitesi, isim/etiket doğruluğu ve gecikme karşılaştırılır.

**Kabul ölçütleri:** Tek/çok konuşmacılı replikler, etiketsiz diyalog, hitap edilen isimler, OCR hatalı etiketler, görev başlıkları ve bozuk yanıtlar denenir. Etiketler iki kez çizilmez; diyalog sırası ve metni korunur. Kalite ve gecikme karşılaştırması yapılmadan özelliğin hız veya doğruluk kazancı varsayılmaz.

## 8. İsteğe Bağlı Sahne İçi Altyazı Dışı Metin Filtreleme (HUD ve Tabelalar)

**Durum: Fikir; henüz uygulanmadı.**

Altyazı yakalama şeridi oyun içi tabelalar, yüzen HUD uyarıları veya arka plan yazılarıyla çakıştığında OCR altyazı dışındaki metinleri de okuyabilir. İsteğe bağlı bir filtre, metni çeviriye göndermeden önce parlaklık ve satır yüksekliği ölçütleriyle ön elemeden geçirebilir.

- Aday OCR satırlarını minimum parlaklık/parlak piksel oranı ve baskın altyazı boyutuna göre beklenen satır yüksekliği aralığıyla süzmek; özelliği varsayılan olarak kapalı ve isteğe bağlı tutmak.
- Sabit koordinat veya oyuna özel kural eklemeden oyun-bağımsız sezgisel ölçütleri korumak.

**Kabul ölçütü:** Gerçek diyalog satırlarını, tek kelimelik yanıtları veya italik/daha sönük konuşmacı ipuçlarını kaybetmeden sönük/küçük arka plan tabelalarını ve farklı ölçekteki HUD etiketlerini eleyebilmek.

## Uygulama Sırası ve Ortak Kurallar

Çeviri geçmişi tamamlandı. Kalan maddelerde önce pencere takibi, ardından replik senkronizasyonu önerilir. Diğer maddeler gerçek kullanımdan gelen ihtiyaçlara göre sıralanabilir.

- Mevcut model, prompt veya bağlam üzerinde değişiklik gerekiyorsa kalite ve gecikme birlikte karşılaştırılır.
- Oyunlara özel sabit koordinatlar veya kurallar eklenmez.
- Yeni ayarlar geriye uyumlu ve varsayılan değerli olur.
- Native Windows API'leri ve mevcut bağımlılıklar tercih edilir.
- Her geliştirme, ilgili işlevsel testler ve kısa bir masaüstü kontrolüyle doğrulanır.
