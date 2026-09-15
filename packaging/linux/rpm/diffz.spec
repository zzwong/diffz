Name:           diffz
Version:        %{diffz_version}
Release:        1%{?dist}
Summary:        Desktop diff and patch viewer
License:        MIT
URL:            https://github.com/zzwong/diffz
BuildArch:      x86_64
BuildRequires:  desktop-file-utils
BuildRequires:  appstream
Requires:       fontconfig
Requires:       dejavu-sans-mono-fonts

%description
Diffz is a desktop application for reviewing diff and patch files.

%install
install -Dm755 %{_sourcedir}/root/usr/bin/diffz \
    %{buildroot}%{_bindir}/diffz
install -Dm644 %{_sourcedir}/root/usr/share/licenses/diffz/LICENSE \
    %{buildroot}%{_licensedir}/diffz/LICENSE
install -Dm644 %{_sourcedir}/root/usr/share/doc/diffz/THIRD_PARTY_NOTICES.md \
    %{buildroot}%{_docdir}/diffz/THIRD_PARTY_NOTICES.md
install -Dm644 %{_sourcedir}/root/usr/share/applications/io.github.zzwong.Diffz.desktop \
    %{buildroot}%{_datadir}/applications/io.github.zzwong.Diffz.desktop
install -Dm644 %{_sourcedir}/root/usr/share/metainfo/io.github.zzwong.Diffz.metainfo.xml \
    %{buildroot}%{_metainfodir}/io.github.zzwong.Diffz.metainfo.xml
install -Dm644 %{_sourcedir}/root/usr/share/icons/hicolor/scalable/apps/io.github.zzwong.Diffz.svg \
    %{buildroot}%{_datadir}/icons/hicolor/scalable/apps/io.github.zzwong.Diffz.svg

%check
desktop-file-validate \
    %{buildroot}%{_datadir}/applications/io.github.zzwong.Diffz.desktop
appstreamcli validate --no-net \
    %{buildroot}%{_metainfodir}/io.github.zzwong.Diffz.metainfo.xml

%files
%{_bindir}/diffz
%license %{_licensedir}/diffz/LICENSE
%doc %{_docdir}/diffz/THIRD_PARTY_NOTICES.md
%{_datadir}/applications/io.github.zzwong.Diffz.desktop
%{_datadir}/metainfo/io.github.zzwong.Diffz.metainfo.xml
%{_datadir}/icons/hicolor/scalable/apps/io.github.zzwong.Diffz.svg
